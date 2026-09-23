# ADR-0014: One clock in the core; the format carries its own locale tables

- Status: Accepted (not yet confirmed by the project owner)
- Date: 2026-09-22

## Context

M3 makes bodies do things: dates and times in a chosen format, the clipboard,
forms, nested snippets, a cursor stop. Its exit criterion is a sentence about
testing rather than about features — *goldens pass under a fixed clock in
three locales* — because an expander that writes the wrong date is worse than
one that writes none.

Two facts pull against each other. `{{date}}` needs the time and the user's
language, which are the two least reproducible things a program can read. And
`aralo-template` has to stay a crate with no files, no network and no clock,
because `scripts/check-deps.sh` enforces that and because the browser
extension will run this same code as WebAssembly, where there is no `libc`
locale and no time zone database.

## Decision

- **The template crate is handed the moment.** `format_time` takes a
  `CivilTime` — year, month, day, hour, minute, second and the offset from UTC
  — and a `&'static Locale`. `render` takes a `Context` that carries both.
  Nothing under `crates/aralo-template/src` reads a clock, and check (e) of
  `check-deps.sh` fails the build if that changes.
- **The one clock lives in `aralo-core`.** `clock::Clock` is a trait with one
  method; `SystemClock` reads `chrono::Local`; `FixedClock(CivilTime)` is
  stopped. The core holds it as an `Arc<dyn Clock>`, so a test hands the core
  a moment instead of waiting for one, and every expansion — the plan, the
  session, the preview — reads the same clock. A shell never implements it:
  outside a test the system clock is the only right answer.
- **Civil time, seconds, an offset and no zone names.** `%z` and `%:z` print
  the offset the clock was running at. There is no `%Z`, because a zone
  *name* means carrying a copy of tzdata into a crate that is not allowed to
  read files. `CivilTime::new` clamps every field into range instead of
  panicking, because a panic behind the UniFFI bridge takes the app down with
  it.
- **The locale tables belong to the format, not to the platform.** Aralo
  carries 15 locales by hand: month and weekday names, long and short, `am`
  and `pm`, and the locale's own `%x` and `%X` patterns. `%c` is `%x`, a
  space, `%X`, so there is no third pattern to keep right. A tag with a region
  Aralo does not carry falls back to the language (`de_AT` reads `de_DE`); a
  language it does not carry falls back to `en_US` **and says so**.
- **The shell tells the core which locale.** A Mac app started from Finder has
  no `LANG`, so `AraloService` calls `Core::set_locale(Locale.current
  .identifier)`. A terminal or a daemon has it in `LC_ALL`, `LC_TIME` or
  `LANG`, which `environment_locale()` reads, skipping `C` and `POSIX`. A body
  may override both with `{{date | locale: fr_FR}}`.
- **An unknown directive is a diagnostic, not a guess.** `{{date: %Q}}` stays
  in the document as written and the editor says which directive it could not
  write. Guessing would expand to nonsense in someone's email.
- **The goldens are the proof.** `fixtures/golden/*.toml` hold the bodies;
  each case is expanded three times — `en_US`, `de_DE` and `ja_JP` — on a
  clock stopped at Monday 9 March 2026, 14:05:07, +01:00, and the transcript
  is compared byte for byte (`ARALO_BLESS=1` takes a wanted change). The three
  are the default, a second language, and a language that is not written in
  Latin letters. The harness also asserts, per case and locale, that the
  editor's preview equals what the expansion inserts (PRD L10).

## Consequences

- A user whose language is not one of the 15 gets English dates and a note
  saying so, rather than a wrong translation. Adding a locale is a table entry
  and a test, which is a pull request anyone can write.
- `{{date}}` reads the same in the preview, in the app, in the CLI, in CI and
  in the browser extension, because the tables travel with the format. It will
  *not* match a system format a user has customised in System Settings; a body
  that wants an exact layout says so with a pattern.
- `cargo build -p aralo-template --target wasm32-unknown-unknown` stays a CI
  gate. Reaching for `chrono` or `icu` inside the template crate breaks two
  checks at once, which is the point.
- Every golden is a fixed-clock expansion, so a date change in the tables
  shows up as a diff in five transcripts rather than as a flaky test.
- Date arithmetic (`{{date | add: 3d}}`) is not in v0 of the grammar. It needs
  calendar rules — month ends, leap years — and it can be added to this crate
  without giving it a clock, so it waits for v1.

## Alternatives considered

- **Format dates through the platform.** `Foundation`'s `DateFormatter` on the
  Mac, `Intl` in the browser, `libc` in the CLI. Rejected: three answers for
  one body, none of them reproducible in a golden file, and no preview in an
  editor that is not the app.
- **Bundle CLDR through `icu`.** Rejected for now: tens of megabytes of data
  and a large dependency tree for a feature whose whole surface is month
  names, weekday names and three patterns. If a user asks for a locale Aralo
  cannot hand-write, this is the decision to revisit, and it supersedes this
  part of the ADR rather than being bolted on.
- **Let `aralo-template` read `SystemTime` when no moment is passed.** Tempting
  and small. Rejected: the convenience is one line, and it costs the guarantee
  that the same body expands the same way twice. A caller that forgets to pass
  a moment should not silently get a different answer in CI than on a desk.
- **Let the shell own the clock and pass a formatted string.** Rejected: the
  format's patterns would then be the shell's, and the second shell would
  write dates differently from the first.
