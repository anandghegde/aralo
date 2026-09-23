//! The moment an expansion happens, and the locale it is written in.
//!
//! `aralo-template` formats a [`CivilTime`] it is handed and has no clock of
//! its own, so this is where the one clock in Aralo lives. Everything that
//! expands takes it from here, which is what lets a test fix the time and the
//! golden files compare byte for byte.

use aralo_template::{default_locale, CivilTime};
use chrono::{DateTime, Datelike, Local, Timelike};

/// Where the core reads the time.
///
/// It is a trait so a test can hand the core a moment instead of waiting for
/// one. A shell never implements it: the system clock is the right answer
/// outside tests.
pub trait Clock: std::fmt::Debug + Send + Sync {
    /// The local date and time now, with the offset from UTC that applies to
    /// it.
    fn now(&self) -> CivilTime;
}

/// The machine's clock, in the machine's time zone.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> CivilTime {
        civil(Local::now())
    }
}

/// A clock stopped at one moment, for tests and the golden files.
#[derive(Debug, Clone, Copy)]
pub struct FixedClock(pub CivilTime);

impl Clock for FixedClock {
    fn now(&self) -> CivilTime {
        self.0
    }
}

/// The local time as the template crate wants it: civil fields, plus the
/// offset `%z` prints. The zone's *name* is deliberately not carried; see
/// `datetime.rs` on `%Z`.
fn civil(now: DateTime<Local>) -> CivilTime {
    let seconds = i64::from(now.offset().local_minus_utc());
    CivilTime::new(
        now.year(),
        now.month() as u8,
        now.day() as u8,
        now.hour() as u8,
        now.minute() as u8,
        now.second() as u8,
    )
    .with_offset((seconds / 60) as i16)
}

/// The locale tag to write dates in, from the environment.
///
/// A terminal or a daemon has it in `LC_ALL`, `LC_TIME` or `LANG`; a macOS app
/// started from Finder usually has none of them, and the shell passes
/// `Locale.current.identifier` to [`Core::set_locale`] instead. `C` and
/// `POSIX` are not languages, so they fall through to the default.
///
/// [`Core::set_locale`]: crate::Core::set_locale
pub fn environment_locale() -> String {
    for name in ["LC_ALL", "LC_TIME", "LANG"] {
        let Ok(value) = std::env::var(name) else {
            continue;
        };
        let tag = value.trim();
        if tag.is_empty() || tag == "C" || tag == "POSIX" {
            continue;
        }
        return tag.to_owned();
    }
    default_locale().tag.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_clock_reports_a_moment_that_formats() {
        let now = SystemClock.now();
        assert!((2024..3000).contains(&now.year()), "{}", now.year());
        assert!((1..=12).contains(&now.month()));
        assert!((-1440..=1440).contains(&now.offset_minutes()));
        let stamp = aralo_template::format_time(now, "%Y-%m-%d %H:%M:%S%:z", default_locale());
        assert!(stamp.is_ok(), "{stamp:?}");
    }

    #[test]
    fn a_fixed_clock_does_not_move() {
        let clock = FixedClock(CivilTime::new(2026, 3, 14, 9, 26, 53));
        assert_eq!(clock.now(), clock.now());
        assert_eq!(clock.now().day(), 14);
    }

    #[test]
    fn the_environment_locale_ignores_the_c_locale() {
        // The process environment is shared, so this test only reads it: what
        // matters is that the answer is a tag the formatter accepts.
        let tag = environment_locale();
        assert!(!tag.is_empty());
        assert_ne!(tag, "C");
        assert_ne!(tag, "POSIX");
    }
}
