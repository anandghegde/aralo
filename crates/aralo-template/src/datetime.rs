//! A civil date and time, and how a locale writes it.
//!
//! The crate has no clock (ADR-0014): a caller passes the local wall-clock
//! time it wants written out, and this module turns it into text with a
//! `strftime` subset. Nothing here reads the system, so a golden file pins an
//! expansion to the minute.
//!
//! The locale tables are the format's, not a shell's, so `{{date | locale: de}}`
//! reads the same on every platform and in the preview. They are hand-kept and
//! deliberately short; [`locales`] is the whole list, and a locale Aralo does
//! not carry falls back to `en_US`.

use std::fmt::Write as _;

/// A date and time as a wall clock shows it: no time zone, no leap seconds.
///
/// [`CivilTime::new`] clamps every field into range, so a caller that
/// miscounts gets the last day of the month rather than a panic behind the
/// bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CivilTime {
    year: i32,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
    offset_minutes: i16,
}

impl CivilTime {
    pub fn new(year: i32, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> Self {
        let month = month.clamp(1, 12);
        let day = day.clamp(1, days_in_month(year, month));
        Self {
            year,
            month,
            day,
            hour: hour.min(23),
            minute: minute.min(59),
            second: second.min(59),
            offset_minutes: 0,
        }
    }

    /// The offset from UTC the clock is running at, for `%z`. Aralo carries no
    /// zone names, so there is no `%Z`.
    #[must_use]
    pub fn with_offset(mut self, offset_minutes: i16) -> Self {
        self.offset_minutes = offset_minutes.clamp(-24 * 60, 24 * 60);
        self
    }

    pub fn year(self) -> i32 {
        self.year
    }

    pub fn month(self) -> u8 {
        self.month
    }

    pub fn day(self) -> u8 {
        self.day
    }

    pub fn hour(self) -> u8 {
        self.hour
    }

    pub fn minute(self) -> u8 {
        self.minute
    }

    pub fn second(self) -> u8 {
        self.second
    }

    pub fn offset_minutes(self) -> i16 {
        self.offset_minutes
    }

    /// Monday is 0, to index the locale tables.
    fn weekday(self) -> usize {
        // 1970-01-01 was a Thursday, index 3 from Monday.
        (days_from_civil(self.year, self.month, self.day) + 3).rem_euclid(7) as usize
    }

    /// 1 on the first of January.
    fn year_day(self) -> u32 {
        let mut days = u32::from(self.day);
        for month in 1..self.month {
            days += u32::from(days_in_month(self.year, month));
        }
        days
    }
}

/// A directive [`format`] does not know, with where it starts in the pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadDirective {
    /// The directive as written, for example `%Q`.
    pub directive: String,
    /// Byte offset of the `%` in the pattern.
    pub at: usize,
}

/// Month and weekday names, and how a locale writes a plain date and time.
///
/// `%c` is always the locale's date, a space, and the locale's time, so there
/// is no third pattern to keep right.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Locale {
    /// The tag this table answers to, for example `de_DE`.
    pub tag: &'static str,
    months: [&'static str; 12],
    months_short: [&'static str; 12],
    /// Monday first.
    weekdays: [&'static str; 7],
    weekdays_short: [&'static str; 7],
    am_pm: [&'static str; 2],
    /// `%x`.
    pub date: &'static str,
    /// `%X`.
    pub time: &'static str,
}

/// Every locale Aralo carries, in the order a menu offers them. The first
/// entry for a language is what a bare language tag such as `de` resolves to.
pub fn locales() -> &'static [Locale] {
    LOCALES
}

/// The locale Aralo uses when nothing else is asked for, and the fallback for
/// a tag it does not carry.
pub fn default_locale() -> &'static Locale {
    &LOCALES[0]
}

/// The table for `tag`, which may be `de`, `de_DE`, `de-DE` or `de_DE.UTF-8`.
///
/// Case and separators do not matter. A tag with a region Aralo does not carry
/// falls back to the language, so `de_AT` gets `de_DE`. Returns `None` for a
/// language that is not in the table: the caller decides whether that is worth
/// saying out loud.
pub fn locale(tag: &str) -> Option<&'static Locale> {
    let tag = tag.trim();
    if tag.is_empty() {
        return None;
    }
    let cut = tag.find(['.', '@']).unwrap_or(tag.len());
    let normalised = tag[..cut].replace('-', "_");
    let mut parts = normalised.split('_');
    let language = parts.next().unwrap_or_default();
    // Region subtags are two letters or three digits; a script subtag such as
    // `Hans` in `zh_Hans_CN` is skipped rather than read as a region.
    let region = parts.find(|part| {
        (part.len() == 2 && part.chars().all(|c| c.is_ascii_alphabetic()))
            || (part.len() == 3 && part.chars().all(|c| c.is_ascii_digit()))
    });

    let matches_language = |entry: &&Locale| {
        entry
            .tag
            .split('_')
            .next()
            .is_some_and(|candidate| candidate.eq_ignore_ascii_case(language))
    };
    if let Some(region) = region {
        let exact = LOCALES.iter().find(|entry| {
            matches_language(entry)
                && entry
                    .tag
                    .split('_')
                    .nth(1)
                    .is_some_and(|candidate| candidate.eq_ignore_ascii_case(region))
        });
        if let Some(exact) = exact {
            return Some(exact);
        }
    }
    LOCALES.iter().find(matches_language)
}

/// Writes `time` the way `pattern` asks for, in `locale`.
///
/// The pattern is a `strftime` subset: see `docs/format/placeholders.md` for
/// the table. An unknown directive is an error rather than a guess, so a
/// snippet that asks for something Aralo cannot write says so in the editor
/// instead of expanding to nonsense.
pub fn format_time(
    time: CivilTime,
    pattern: &str,
    locale: &'static Locale,
) -> Result<String, BadDirective> {
    let mut out = String::with_capacity(pattern.len() + 16);
    write_pattern(&mut out, time, pattern, locale, 0, 0)?;
    Ok(out)
}

/// `depth` guards the locale patterns, which are patterns themselves; `base`
/// keeps a bad directive's offset pointing into the pattern the user wrote.
fn write_pattern(
    out: &mut String,
    time: CivilTime,
    pattern: &str,
    locale: &'static Locale,
    depth: u8,
    base: usize,
) -> Result<(), BadDirective> {
    let mut rest = pattern;
    let mut at = 0;
    while let Some(percent) = rest.find('%') {
        out.push_str(&rest[..percent]);
        at += percent;
        let after = &rest[percent + 1..];
        let mut chars = after.char_indices();
        let (pad, letter, letter_end) = match chars.next() {
            Some((_, flag @ ('-' | '_' | '0'))) => match chars.next() {
                Some((offset, letter)) => (Some(flag), letter, offset + letter.len_utf8()),
                None => {
                    // A pattern ending in `%-`: nothing to pad.
                    return Err(BadDirective {
                        directive: format!("%{flag}"),
                        at: base + at,
                    });
                }
            },
            Some((offset, letter)) => (None, letter, offset + letter.len_utf8()),
            None => {
                return Err(BadDirective {
                    directive: "%".to_owned(),
                    at: base + at,
                })
            }
        };
        let width = |default: usize| match pad {
            Some('-') => Pad::None,
            Some('_') => Pad::Space(default),
            _ => Pad::Zero(default),
        };
        match letter {
            'Y' => write_number(out, i64::from(time.year), width(4)),
            'y' => write_number(out, i64::from(time.year).rem_euclid(100), width(2)),
            'm' => write_number(out, i64::from(time.month), width(2)),
            'd' => write_number(out, i64::from(time.day), width(2)),
            'e' => write_number(out, i64::from(time.day), Pad::Space(2)),
            'H' => write_number(out, i64::from(time.hour), width(2)),
            'k' => write_number(out, i64::from(time.hour), Pad::Space(2)),
            'I' => write_number(out, i64::from(twelve_hour(time.hour)), width(2)),
            'l' => write_number(out, i64::from(twelve_hour(time.hour)), Pad::Space(2)),
            'M' => write_number(out, i64::from(time.minute), width(2)),
            'S' => write_number(out, i64::from(time.second), width(2)),
            'j' => write_number(out, i64::from(time.year_day()), width(3)),
            'u' => write_number(out, time.weekday() as i64 + 1, Pad::None),
            'w' => write_number(out, (time.weekday() as i64 + 1) % 7, Pad::None),
            'B' => out.push_str(locale.months[usize::from(time.month) - 1]),
            'b' | 'h' => out.push_str(locale.months_short[usize::from(time.month) - 1]),
            'A' => out.push_str(locale.weekdays[time.weekday()]),
            'a' => out.push_str(locale.weekdays_short[time.weekday()]),
            'p' => out.push_str(locale.am_pm[usize::from(time.hour >= 12)]),
            'P' => out.push_str(&locale.am_pm[usize::from(time.hour >= 12)].to_lowercase()),
            'z' | ':' if letter == 'z' || after[letter_end..].starts_with('z') => {
                let colon = letter == ':';
                let minutes = i64::from(time.offset_minutes);
                let sign = if minutes < 0 { '-' } else { '+' };
                let minutes = minutes.abs();
                out.push(sign);
                write_number(out, minutes / 60, Pad::Zero(2));
                if colon {
                    out.push(':');
                }
                write_number(out, minutes % 60, Pad::Zero(2));
            }
            'n' => out.push('\n'),
            't' => out.push('\t'),
            '%' => out.push('%'),
            'x' | 'X' | 'c' | 'F' | 'T' | 'R' | 'D' | 'r' if depth < 2 => {
                let nested = match letter {
                    'x' => locale.date,
                    'X' => locale.time,
                    'c' => {
                        write_pattern(out, time, locale.date, locale, depth + 1, base + at)?;
                        out.push(' ');
                        locale.time
                    }
                    'F' => "%Y-%m-%d",
                    'T' => "%H:%M:%S",
                    'R' => "%H:%M",
                    'D' => "%m/%d/%y",
                    _ => "%I:%M:%S %p",
                };
                write_pattern(out, time, nested, locale, depth + 1, base + at)?;
            }
            other => {
                let mut directive = String::from("%");
                if let Some(pad) = pad {
                    directive.push(pad);
                }
                directive.push(other);
                return Err(BadDirective {
                    directive,
                    at: base + at,
                });
            }
        }
        // `%:z` eats one more character than the match arm saw.
        let consumed = if letter == ':' {
            letter_end + 1
        } else {
            letter_end
        };
        rest = &after[consumed..];
        at += 1 + consumed;
    }
    out.push_str(rest);
    Ok(())
}

enum Pad {
    None,
    Zero(usize),
    Space(usize),
}

fn write_number(out: &mut String, value: i64, pad: Pad) {
    let _ = match pad {
        Pad::None => write!(out, "{value}"),
        Pad::Zero(width) if value < 0 => write!(out, "-{:0width$}", -value, width = width),
        Pad::Zero(width) => write!(out, "{value:0width$}"),
        Pad::Space(width) => write!(out, "{value:width$}"),
    };
}

fn twelve_hour(hour: u8) -> u8 {
    match hour % 12 {
        0 => 12,
        other => other,
    }
}

fn is_leap(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => 31,
    }
}

/// Days from 1970-01-01 to this date, Howard Hinnant's `days_from_civil`.
fn days_from_civil(year: i32, month: u8, day: u8) -> i64 {
    let year = i64::from(year) - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month = i64::from(month);
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

// Nine fields make one locale, and naming them at fifteen call sites is worse
// than the lint it trips.
#[allow(clippy::too_many_arguments)]
const fn table(
    tag: &'static str,
    months: [&'static str; 12],
    months_short: [&'static str; 12],
    weekdays: [&'static str; 7],
    weekdays_short: [&'static str; 7],
    am_pm: [&'static str; 2],
    date: &'static str,
    time: &'static str,
) -> Locale {
    Locale {
        tag,
        months,
        months_short,
        weekdays,
        weekdays_short,
        am_pm,
        date,
        time,
    }
}

const TWENTY_FOUR: &str = "%H:%M:%S";

const LOCALES: &[Locale] = &[
    table(
        "en_US",
        [
            "January",
            "February",
            "March",
            "April",
            "May",
            "June",
            "July",
            "August",
            "September",
            "October",
            "November",
            "December",
        ],
        [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ],
        [
            "Monday",
            "Tuesday",
            "Wednesday",
            "Thursday",
            "Friday",
            "Saturday",
            "Sunday",
        ],
        ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"],
        ["AM", "PM"],
        "%m/%d/%Y",
        "%I:%M:%S %p",
    ),
    table(
        "en_GB",
        [
            "January",
            "February",
            "March",
            "April",
            "May",
            "June",
            "July",
            "August",
            "September",
            "October",
            "November",
            "December",
        ],
        [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ],
        [
            "Monday",
            "Tuesday",
            "Wednesday",
            "Thursday",
            "Friday",
            "Saturday",
            "Sunday",
        ],
        ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"],
        ["am", "pm"],
        "%d/%m/%Y",
        TWENTY_FOUR,
    ),
    table(
        "de_DE",
        [
            "Januar",
            "Februar",
            "März",
            "April",
            "Mai",
            "Juni",
            "Juli",
            "August",
            "September",
            "Oktober",
            "November",
            "Dezember",
        ],
        [
            "Jan", "Feb", "Mär", "Apr", "Mai", "Jun", "Jul", "Aug", "Sep", "Okt", "Nov", "Dez",
        ],
        [
            "Montag",
            "Dienstag",
            "Mittwoch",
            "Donnerstag",
            "Freitag",
            "Samstag",
            "Sonntag",
        ],
        ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"],
        ["vorm.", "nachm."],
        "%d.%m.%Y",
        TWENTY_FOUR,
    ),
    table(
        "fr_FR",
        [
            "janvier",
            "février",
            "mars",
            "avril",
            "mai",
            "juin",
            "juillet",
            "août",
            "septembre",
            "octobre",
            "novembre",
            "décembre",
        ],
        [
            "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.",
            "nov.", "déc.",
        ],
        [
            "lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi", "dimanche",
        ],
        ["lun.", "mar.", "mer.", "jeu.", "ven.", "sam.", "dim."],
        ["AM", "PM"],
        "%d/%m/%Y",
        TWENTY_FOUR,
    ),
    table(
        "es_ES",
        [
            "enero",
            "febrero",
            "marzo",
            "abril",
            "mayo",
            "junio",
            "julio",
            "agosto",
            "septiembre",
            "octubre",
            "noviembre",
            "diciembre",
        ],
        [
            "ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sep", "oct", "nov", "dic",
        ],
        [
            "lunes",
            "martes",
            "miércoles",
            "jueves",
            "viernes",
            "sábado",
            "domingo",
        ],
        ["lun", "mar", "mié", "jue", "vie", "sáb", "dom"],
        ["a. m.", "p. m."],
        "%d/%m/%Y",
        TWENTY_FOUR,
    ),
    table(
        "it_IT",
        [
            "gennaio",
            "febbraio",
            "marzo",
            "aprile",
            "maggio",
            "giugno",
            "luglio",
            "agosto",
            "settembre",
            "ottobre",
            "novembre",
            "dicembre",
        ],
        [
            "gen", "feb", "mar", "apr", "mag", "giu", "lug", "ago", "set", "ott", "nov", "dic",
        ],
        [
            "lunedì",
            "martedì",
            "mercoledì",
            "giovedì",
            "venerdì",
            "sabato",
            "domenica",
        ],
        ["lun", "mar", "mer", "gio", "ven", "sab", "dom"],
        ["AM", "PM"],
        "%d/%m/%Y",
        TWENTY_FOUR,
    ),
    table(
        "pt_BR",
        [
            "janeiro",
            "fevereiro",
            "março",
            "abril",
            "maio",
            "junho",
            "julho",
            "agosto",
            "setembro",
            "outubro",
            "novembro",
            "dezembro",
        ],
        [
            "jan", "fev", "mar", "abr", "mai", "jun", "jul", "ago", "set", "out", "nov", "dez",
        ],
        [
            "segunda-feira",
            "terça-feira",
            "quarta-feira",
            "quinta-feira",
            "sexta-feira",
            "sábado",
            "domingo",
        ],
        ["seg", "ter", "qua", "qui", "sex", "sáb", "dom"],
        ["AM", "PM"],
        "%d/%m/%Y",
        TWENTY_FOUR,
    ),
    table(
        "nl_NL",
        [
            "januari",
            "februari",
            "maart",
            "april",
            "mei",
            "juni",
            "juli",
            "augustus",
            "september",
            "oktober",
            "november",
            "december",
        ],
        [
            "jan", "feb", "mrt", "apr", "mei", "jun", "jul", "aug", "sep", "okt", "nov", "dec",
        ],
        [
            "maandag",
            "dinsdag",
            "woensdag",
            "donderdag",
            "vrijdag",
            "zaterdag",
            "zondag",
        ],
        ["ma", "di", "wo", "do", "vr", "za", "zo"],
        ["a.m.", "p.m."],
        "%d-%m-%Y",
        TWENTY_FOUR,
    ),
    table(
        "sv_SE",
        [
            "januari",
            "februari",
            "mars",
            "april",
            "maj",
            "juni",
            "juli",
            "augusti",
            "september",
            "oktober",
            "november",
            "december",
        ],
        [
            "jan", "feb", "mar", "apr", "maj", "jun", "jul", "aug", "sep", "okt", "nov", "dec",
        ],
        [
            "måndag", "tisdag", "onsdag", "torsdag", "fredag", "lördag", "söndag",
        ],
        ["mån", "tis", "ons", "tors", "fre", "lör", "sön"],
        ["fm", "em"],
        "%Y-%m-%d",
        TWENTY_FOUR,
    ),
    table(
        "da_DK",
        [
            "januar",
            "februar",
            "marts",
            "april",
            "maj",
            "juni",
            "juli",
            "august",
            "september",
            "oktober",
            "november",
            "december",
        ],
        [
            "jan", "feb", "mar", "apr", "maj", "jun", "jul", "aug", "sep", "okt", "nov", "dec",
        ],
        [
            "mandag", "tirsdag", "onsdag", "torsdag", "fredag", "lørdag", "søndag",
        ],
        ["man", "tir", "ons", "tor", "fre", "lør", "søn"],
        ["AM", "PM"],
        "%d.%m.%Y",
        TWENTY_FOUR,
    ),
    table(
        "nb_NO",
        [
            "januar",
            "februar",
            "mars",
            "april",
            "mai",
            "juni",
            "juli",
            "august",
            "september",
            "oktober",
            "november",
            "desember",
        ],
        [
            "jan", "feb", "mar", "apr", "mai", "jun", "jul", "aug", "sep", "okt", "nov", "des",
        ],
        [
            "mandag", "tirsdag", "onsdag", "torsdag", "fredag", "lørdag", "søndag",
        ],
        ["man", "tir", "ons", "tor", "fre", "lør", "søn"],
        ["AM", "PM"],
        "%d.%m.%Y",
        TWENTY_FOUR,
    ),
    table(
        "tr_TR",
        [
            "Ocak", "Şubat", "Mart", "Nisan", "Mayıs", "Haziran", "Temmuz", "Ağustos", "Eylül",
            "Ekim", "Kasım", "Aralık",
        ],
        [
            "Oca", "Şub", "Mar", "Nis", "May", "Haz", "Tem", "Ağu", "Eyl", "Eki", "Kas", "Ara",
        ],
        [
            "Pazartesi",
            "Salı",
            "Çarşamba",
            "Perşembe",
            "Cuma",
            "Cumartesi",
            "Pazar",
        ],
        ["Pzt", "Sal", "Çar", "Per", "Cum", "Cmt", "Paz"],
        ["ÖÖ", "ÖS"],
        "%d.%m.%Y",
        TWENTY_FOUR,
    ),
    table(
        "ja_JP",
        [
            "1月", "2月", "3月", "4月", "5月", "6月", "7月", "8月", "9月", "10月", "11月", "12月",
        ],
        [
            "1月", "2月", "3月", "4月", "5月", "6月", "7月", "8月", "9月", "10月", "11月", "12月",
        ],
        [
            "月曜日",
            "火曜日",
            "水曜日",
            "木曜日",
            "金曜日",
            "土曜日",
            "日曜日",
        ],
        ["月", "火", "水", "木", "金", "土", "日"],
        ["午前", "午後"],
        "%Y年%m月%d日",
        TWENTY_FOUR,
    ),
    table(
        "zh_CN",
        [
            "一月",
            "二月",
            "三月",
            "四月",
            "五月",
            "六月",
            "七月",
            "八月",
            "九月",
            "十月",
            "十一月",
            "十二月",
        ],
        [
            "1月", "2月", "3月", "4月", "5月", "6月", "7月", "8月", "9月", "10月", "11月", "12月",
        ],
        [
            "星期一",
            "星期二",
            "星期三",
            "星期四",
            "星期五",
            "星期六",
            "星期日",
        ],
        ["周一", "周二", "周三", "周四", "周五", "周六", "周日"],
        ["上午", "下午"],
        "%Y年%m月%d日",
        TWENTY_FOUR,
    ),
    table(
        "ko_KR",
        [
            "1월", "2월", "3월", "4월", "5월", "6월", "7월", "8월", "9월", "10월", "11월", "12월",
        ],
        [
            "1월", "2월", "3월", "4월", "5월", "6월", "7월", "8월", "9월", "10월", "11월", "12월",
        ],
        [
            "월요일",
            "화요일",
            "수요일",
            "목요일",
            "금요일",
            "토요일",
            "일요일",
        ],
        ["월", "화", "수", "목", "금", "토", "일"],
        ["오전", "오후"],
        "%Y년 %m월 %d일",
        TWENTY_FOUR,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Monday 9 March 2026, 14:05:07.
    fn monday() -> CivilTime {
        CivilTime::new(2026, 3, 9, 14, 5, 7)
    }

    fn en(pattern: &str) -> String {
        format_time(monday(), pattern, default_locale()).expect("known directives")
    }

    #[test]
    fn numbers_pad_the_way_strftime_pads() {
        assert_eq!(en("%Y-%m-%d %H:%M:%S"), "2026-03-09 14:05:07");
        assert_eq!(en("%-m/%-d/%Y"), "3/9/2026");
        assert_eq!(en("%e|%k|%l"), " 9|14| 2");
        assert_eq!(en("%y %j %u %w"), "26 068 1 1");
        assert_eq!(en("%I:%M %p"), "02:05 PM");
        assert_eq!(en("%-I:%M %P"), "2:05 pm");
    }

    #[test]
    fn names_come_from_the_locale() {
        assert_eq!(en("%A %-d %B %Y"), "Monday 9 March 2026");
        let german = locale("de").expect("carried");
        assert_eq!(
            format_time(monday(), "%A, %-d. %B %Y", german).unwrap(),
            "Montag, 9. März 2026"
        );
        let japanese = locale("ja_JP").expect("carried");
        assert_eq!(
            format_time(monday(), "%x (%a)", japanese).unwrap(),
            "2026年03月09日 (月)"
        );
    }

    #[test]
    fn the_locale_writes_a_plain_date_and_time() {
        assert_eq!(en("%x %X"), "03/09/2026 02:05:07 PM");
        assert_eq!(en("%c"), "03/09/2026 02:05:07 PM");
        assert_eq!(
            format_time(monday(), "%x %X", locale("sv").unwrap()).unwrap(),
            "2026-03-09 14:05:07"
        );
        assert_eq!(
            en("%F %T %R %D %r"),
            "2026-03-09 14:05:07 14:05 03/09/26 02:05:07 PM"
        );
    }

    #[test]
    fn literal_text_and_escapes_survive() {
        assert_eq!(en("100%% sure%nnext%ttab"), "100% sure\nnext\ttab");
        assert_eq!(en("week of %d"), "week of 09");
        assert_eq!(en(""), "");
        assert_eq!(en("no directives"), "no directives");
    }

    #[test]
    fn an_unknown_directive_says_which_one_and_where() {
        let error = format_time(monday(), "%Y-%Q", default_locale()).expect_err("no %Q");
        assert_eq!(error.directive, "%Q");
        assert_eq!(error.at, 3);
        assert_eq!(
            format_time(monday(), "%Z", default_locale())
                .expect_err("no zone names")
                .directive,
            "%Z"
        );
        // A pattern that ends mid-directive is an error, not a panic.
        assert_eq!(
            format_time(monday(), "ends %", default_locale())
                .expect_err("cut off")
                .at,
            5
        );
        assert_eq!(
            format_time(monday(), "ends %-", default_locale())
                .expect_err("cut off")
                .at,
            5
        );
    }

    #[test]
    fn the_utc_offset_is_written_only_when_asked_for() {
        let time = monday().with_offset(-330);
        assert_eq!(
            format_time(time, "%z %:z", default_locale()).unwrap(),
            "-0530 -05:30"
        );
        assert_eq!(
            format_time(monday(), "%z", default_locale()).unwrap(),
            "+0000"
        );
    }

    #[test]
    fn locale_tags_are_forgiving() {
        for tag in [
            "de",
            "de_DE",
            "de-DE",
            "de_AT",
            "DE_de",
            "de_DE.UTF-8",
            " de ",
        ] {
            assert_eq!(locale(tag).map(|l| l.tag), Some("de_DE"), "{tag}");
        }
        assert_eq!(locale("en").map(|l| l.tag), Some("en_US"));
        assert_eq!(locale("en_GB").map(|l| l.tag), Some("en_GB"));
        assert_eq!(locale("zh_Hans_CN").map(|l| l.tag), Some("zh_CN"));
        assert_eq!(locale("hi_IN"), None);
        assert_eq!(locale(""), None);
        assert_eq!(default_locale().tag, "en_US");
    }

    #[test]
    fn weekdays_and_leap_years_are_counted_not_guessed() {
        // 2000-02-29 was a Tuesday; 1900 was not a leap year, so 1900-02-29
        // clamps to the 28th, a Wednesday.
        assert_eq!(
            format_time(
                CivilTime::new(2000, 2, 29, 0, 0, 0),
                "%A %j",
                default_locale()
            )
            .unwrap(),
            "Tuesday 060"
        );
        assert_eq!(
            format_time(
                CivilTime::new(1900, 2, 29, 0, 0, 0),
                "%A %d %j",
                default_locale()
            )
            .unwrap(),
            "Wednesday 28 059"
        );
        assert_eq!(
            format_time(
                CivilTime::new(2026, 12, 31, 23, 59, 59),
                "%A %j",
                default_locale()
            )
            .unwrap(),
            "Thursday 365"
        );
        assert_eq!(
            format_time(
                CivilTime::new(1969, 7, 20, 20, 17, 40),
                "%A %x",
                default_locale()
            )
            .unwrap(),
            "Sunday 07/20/1969"
        );
    }

    #[test]
    fn out_of_range_fields_clamp_instead_of_panicking() {
        let time = CivilTime::new(2026, 99, 99, 99, 99, 99);
        assert_eq!(
            format_time(time, "%Y-%m-%d %H:%M:%S", default_locale()).unwrap(),
            "2026-12-31 23:59:59"
        );
        assert_eq!(CivilTime::new(2026, 0, 0, 0, 0, 0).month(), 1);
        assert_eq!(CivilTime::new(2026, 4, 31, 0, 0, 0).day(), 30);
        assert_eq!(monday().with_offset(i16::MAX).offset_minutes(), 1440);
    }

    #[test]
    fn every_locale_writes_its_own_plain_date_and_time() {
        for entry in locales() {
            for pattern in [entry.date, entry.time, "%c", "%A %B %a %b %p"] {
                let written = format_time(monday(), pattern, entry).unwrap_or_else(|error| {
                    panic!("{} cannot write {pattern}: {error:?}", entry.tag)
                });
                assert!(!written.is_empty(), "{} wrote nothing", entry.tag);
                assert!(
                    !written.contains('%'),
                    "{} left a directive in {written}",
                    entry.tag
                );
            }
            assert_eq!(locale(entry.tag).map(|l| l.tag), Some(entry.tag));
        }
    }
}
