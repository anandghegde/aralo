# Matching

This document defines when typed keys expand a snippet, which snippet wins,
how much is deleted, and when the engine forgets what it has seen.

The reference implementation is `crates/aralo-engine`. Every rule here is
covered by `crates/aralo-engine/tests/matching.rs`, and the trie matcher is
property-tested against a naive matcher in `tests/reference.rs`.

All lengths in this document are counted in `char`s (Unicode scalar values),
not bytes and not grapheme clusters.

## Inputs

The engine sees three kinds of key event from the shell:

| Event | Meaning |
| --- | --- |
| `Char(c)` | The key produced the character `c` |
| `Backspace` | Backspace was pressed |
| `Undo` | The platform's undo shortcut was pressed |

It also hears about state changes: the front app, pause, the excluded-app
list, a new set of snippets, and resets (below). It never sees key codes,
modifier state or window contents.

## The buffer

The engine keeps the most recent typed characters in a fixed ring of
**64 characters**. It holds nothing else about what was typed, except the
one-key undo record described below.

- A character is pushed for every `Char`. When the ring is full the oldest
  character is overwritten.
- `Backspace` removes the newest character.
- Every slot that stops holding a live character is zeroed. The whole ring is
  zeroed on every reset, after every match, and when the engine is dropped.

An abbreviation may be at most **63 characters**, one less than the ring, so
that the character before a candidate match is always available for the
whole-word rule. An abbreviation that is empty or longer than 63 characters is
rejected when the snippet set is built. The rejection is reported and the rest
of the set still loads.

## Per-abbreviation settings

Each abbreviation carries these settings. They come from the snippet's front
matter and the group defaults; see [snippet.md](snippet.md) and
[library.md](library.md#inheritance).

| Setting | Values | Built-in default |
| --- | --- | --- |
| `trigger` | `immediate`, `delimiter` | `delimiter` |
| `case` | `exact`, `ignore`, `adaptive` | `adaptive` |
| `word` | `true`, `false` | `true` |
| `keep_delimiter` | `true`, `false` | `true` |
| delimiters | a set of characters (group-level `defaults.delimiters`) | the default set below |
| scope | everywhere, only these apps, all but these apps | everywhere |

## Triggers

**`immediate`** expands as soon as the last character of the abbreviation is
typed.

**`delimiter`** expands when a delimiter is typed directly after the
abbreviation. The delimiter must belong to that abbreviation's own delimiter
set.

For each `Char(c)` the engine runs two passes:

1. **Delimiter pass, before `c` is pushed.** If `c` could be a delimiter, the
   engine looks for a `delimiter` abbreviation that ends at the end of the
   buffer and accepts `c`.
2. **Immediate pass, after `c` is pushed.** The engine looks for an
   `immediate` abbreviation that ends with `c`.

If both passes find a candidate, the tie-break rules below decide.

### Default delimiters

The built-in delimiter set has 23 characters:

| Kind | Characters |
| --- | --- |
| White space | space, tab, line feed, carriage return |
| Punctuation | `.` `,` `;` `:` `!` `?` |
| Brackets | `(` `)` `[` `]` `{` `}` `<` `>` |
| Other | `/` `\` `'` `"` `-` |

A group can replace the set with `defaults.delimiters`. Every character of
that string is a delimiter. A snippet cannot set its own delimiter set.

A delimiter character may also appear inside an abbreviation. `;rf` works
with the default set: the `;` is matched as part of the abbreviation, and a
later delimiter triggers it.

## Case

Matching works on **folded** characters. A character folds to its lower-case
form when that form is exactly one character; otherwise it folds to itself.
So `A` folds to `a`, while `İ` and `ß` stay as they are. Both the defined
abbreviation and the typed text are folded the same way.

| `case` | Matches when | Effect on the expansion |
| --- | --- | --- |
| `exact` | The typed text equals the defined abbreviation, character for character | None |
| `ignore` | The folded forms are equal | None |
| `adaptive` | The folded forms are equal | The typed case pattern re-cases the expansion |

### Adaptive case pattern

The pattern is read from the typed abbreviation. Only cased letters count;
digits and punctuation are ignored.

| Typed | Pattern | Rule |
| --- | --- | --- |
| Exactly as defined | as defined | Checked first |
| No cased letters | as defined | |
| First cased letter is not upper case (`tY`) | as defined | |
| First cased letter upper, at least one more cased letter, all of them upper (`;RF`) | UPPER | |
| Anything else that starts with an upper-case letter (`;Rf`, a single `A`) | Title | |

The pattern is applied to the expansion by `aralo-template`:

| Pattern | Transform |
| --- | --- |
| as defined | None |
| Title | The first alphabetic character is upper-cased. The rest is unchanged |
| UPPER | The whole expansion is upper-cased |

## Whole word

With `word: true`, an abbreviation matches only when the character directly
before it is **not** a word character. A word character is any alphanumeric
character or `_`. The start of the buffer counts as a boundary, so an
abbreviation typed straight after a reset matches.

With `word: false` the abbreviation matches anywhere, including in the middle
of a word.

## Scope

A scope is one of:

| Scope | Matches |
| --- | --- |
| everywhere | Any front app |
| only | The front app is in the list |
| except | The front app is not in the list |

Entries are app identifiers: bundle IDs on macOS. They are compared without
regard to ASCII case.

Until the shell has named a front app, the app identifier is the empty
string. An `only` scope then matches nothing and an `except` scope matches.

## Tie-breaks

More than one abbreviation can be a candidate for the same key. The engine
picks exactly one, in this order:

1. **Longest abbreviation wins.**
2. **Then the most specific scope:** `only` beats `except`, which beats
   everywhere.
3. **Then `immediate` beats `delimiter`,** when a candidate from each pass is
   equal on the first two.
4. **Then the lowest snippet ID.** IDs are ULIDs, so this is normally the
   snippet created first.

The result is deterministic: the same library and the same keys always pick
the same snippet.

## The verdict

For every key the engine returns one verdict.

| Verdict | Meaning |
| --- | --- |
| `Pass` | Nothing to do. The key goes to the app untouched |
| `Match` | Expand a snippet |
| `UndoLast` | Undo the expansion that was just made |

`Match` carries:

| Field | Meaning |
| --- | --- |
| `snippet_id` | The snippet to expand |
| `delete_count` | How many characters already in the document to delete |
| `consume` | The shell must swallow the triggering key. Always `true` today |
| `case` | The adaptive case pattern: as defined, Title or UPPER |
| `trailing` | The delimiter to re-insert after the expansion, if any |

Because the triggering key is consumed, it never reaches the document. That
fixes `delete_count`:

| Trigger | `delete_count` | `trailing` |
| --- | --- | --- |
| `immediate` | Abbreviation length minus one. The last character was consumed | none |
| `delimiter` | Abbreviation length. The delimiter was consumed | The delimiter, if `keep_delimiter` is `true` |

After a match the buffer is zeroed.

## Undo

Directly after an expansion, one `Backspace` or `Undo` reverses it.

1. The shell reports a finished expansion with `expansion_done`, giving the
   snippet ID, how many characters were inserted, and whether they were typed
   or pasted.
2. If the very next key is `Backspace` or `Undo`, the verdict is `UndoLast`
   with the number of characters to delete, the text to retype and the insert
   method. The text to retype is what the user originally typed, including the
   delimiter for a delimiter trigger.
3. Any other key, or any reset, drops the undo record.

`expansion_done` is ignored if another key arrived between the match and the
report, or if the snippet ID is not the one that matched.

The typed text is held for that single key and then zeroed. It is the only
typed text the engine holds outside the ring.

## Resets

A reset zeroes the ring and drops the undo record. The shell sends one
whenever the text before the cursor can no longer be assumed:

| Reason | Sent when |
| --- | --- |
| `MouseDown` | A mouse button went down |
| `Navigation` | Arrow keys, Home, End, Page Up and the like |
| `Shortcut` | A key combination with Command or Control |
| `AppSwitch` | The front app changed |
| `FocusChange` | Focus moved |
| `SecureInput` | Secure input is on |
| `InputMethod` | An input method started composing. Full input-method support is planned for v1 |
| `UnmappableInput` | A key event did not map to exactly one character |
| `Manual` | The shell asked for it |

Every reason does the same thing. The reason exists so that the shell's code
reads clearly and the privacy tests can name each case.

These state changes also reset: a new front app, pause on or off, a new
excluded-app list, a new snippet set.

## Pause and excluded apps

While the engine is paused, or the front app is on the excluded list, every
key returns `Pass`, nothing is pushed to the ring, and the ring stays zeroed.

The excluded list is supplied by the shell. The engine ships a preset list of
password managers for the shell to include:

```
com.1password.1password
com.agilebits.onepassword7
com.apple.keychainaccess
com.apple.Passwords
com.bitwarden.desktop
org.keepassxc.keepassxc
```

Setting the list replaces it. A shell that wants the presets includes them.

## What the engine does not do

- It does not read files, the clock, the clipboard or the network. It is
  `#![no_std]` and depends on one crate, `zeroize`.
- It does not log. `println!`, `eprintln!`, `dbg!`, `log::` and `tracing::`
  are rejected in `crates/aralo-engine/src` by `scripts/check-deps.sh`.
- Its `Debug` output carries counts only, never typed text.

See [docs/architecture.md](../architecture.md#privacy-invariants) for how
each of these is enforced.
