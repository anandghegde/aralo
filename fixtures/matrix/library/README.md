# Injection matrix library

The snippets the injection matrix types in every app
(`apps/macos/Packages/AraloKit/Sources/AraloHarness`). The harness starts
Aralo on this folder, so nothing from a real library is ever typed into a test
app. The abbreviations are lower-case letters only, so that they sit on
unshifted keys in every Latin keyboard layout.

The harness never reads the bodies here. It asks the core what each
abbreviation expands to and compares that with what the app shows.

This file has no front matter, so the loader reports it as "not a snippet".
