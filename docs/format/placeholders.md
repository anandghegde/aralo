# Placeholders

A snippet body is a template. Text is inserted as written. `{{…}}`
placeholders are replaced when the snippet expands.

The reference implementation is `crates/aralo-template`.

**Status.** The parser and the evaluator are implemented and tested. Dates and
times, the clipboard, form fields, drop-downs, nested snippets and cursor
stops expand. An AI block inserts its `fallback:` text until models arrive
(M4). `{{selection}}`, `{{app}}` and `{{window}}` stay as written for now, with
a note saying so. `{{if}}` blocks are format v1 and are not parsed. Every
section below says which of the two it is.

A placeholder Aralo cannot expand is inserted as its own source text, so
nothing is silently dropped: `{{nonsense}}` expands to the literal characters
`{{nonsense}}`, and the editor says why.

## Grammar

```text
body        := (text | placeholder | block)*
placeholder := "{{" name (":" argument)? ("|" option)* "}}"
option      := key ":" value
block       := "{{if: " condition "}}" body ("{{else}}" body)? "{{end}}"     # v1
escape      := "\{{"  ->  literal "{{"
```

`block` is planned for v1 (PRD D6) and is not parsed today. Today
`{{if: x}}`, `{{else}}` and `{{end}}` parse as ordinary placeholders named
`if`, `else` and `end`, and expand to themselves.

## Parsing rules (implemented)

1. **A placeholder starts at `{{` and ends at the first `}}` after it.**
   Placeholders do not nest, and there is no way to write `}}` inside one.
2. **`\{{` is a literal `{{`.** It is the only escape in body text. A
   backslash anywhere else is an ordinary character, and there is no escape
   for the backslash itself. A lone `{`, a lone `}` and a `}}` with no opening
   `{{` are ordinary text.
3. **The name** comes first. It must match `[a-z][a-z0-9_-]*`: lower-case
   ASCII, starting with a letter. White space around it is ignored.
4. **The argument** is everything after the first `:` in the head, up to the
   first unescaped `|`. It is trimmed. An empty argument is the same as none.
   Only the first `:` splits, so `{{time: %H:%M}}` has the argument `%H:%M`.
5. **Options** follow, separated by `|`. Each is `key: value`, split on the
   first `:`. The key follows the same rule as a name. Key and value are
   trimmed. Options keep their source order, and a repeated key keeps every
   value; a reader that wants one value takes the first.
6. **`\|` is a literal pipe** inside a placeholder, so an AI prompt can
   contain one.

```text
{{field: name | label: Customer name | default: there}}
  ^name  ^argument  ^option                ^option
```

## Expanding happens in two steps

The middle of an expansion belongs to the shell, so the evaluator is split
where the waiting is:

1. **Resolve** reads the body, inlines the snippets it nests, collects the
   form it wants filled in, and says which context kinds it needs. It takes no
   clipboard and no answers, so an editor can run it on every keystroke and
   show everything that is wrong with a body before anything is typed.
2. **Render** turns that into text, with the clock, the answers and whatever
   context the shell fetched.

A shell asks in a fixed order: the form first, then the context, then the
expansion (`crates/aralo-core/src/session.rs`). **A kind the body does not ask
for is never requested from the shell** — not merely never sent (PRD P4).

A preview is a real expansion: it renders the body with the form's defaults
and no context, which is why what the editor shows is what goes in (PRD L10).

## Placeholders

| Placeholder | Behaviour | Status | PRD |
| --- | --- | --- | --- |
| `{{date}}`, `{{date: format \| locale: de}}` | The current date. Arithmetic such as `\| add: 3d` is planned for v1 | Implemented | D1 |
| `{{time}}`, `{{time: format}}` | The current time | Implemented | D2 |
| `{{clipboard}}` | The clipboard as plain text. History slots are planned for v1 | Implemented | D3 |
| `{{field: name \| label: x \| default: y \| lines: 4}}` | A box to type in, `lines:` tall | Implemented | D4 |
| `{{choice: name \| options: a, b, c \| default: b}}` | A drop-down | Implemented | D4 |
| `{{snippet: name-or-id}}` | Another snippet, inlined | Implemented | D7 |
| `{{cursor}}`, `{{cursor: select}}` | Where the caret is left | Implemented | E5 |
| `{{key: tab}}`, `{{key: return}}` | A key the app acts on, not a character | Implemented | — |
| `{{ai: prompt \| fallback: text \| model: x}}` | An AI block | Inserts `fallback` (M4) | D15, A2 |
| `{{selection}}`, `{{app}}`, `{{window}}` | The selection, the front app, its window title | Stays as written | P4, D12 |

In this table `\|` is only how a pipe is written inside a Markdown table cell.
In a snippet body write a plain `|`.

Which names are known, and what an editor's insert menu offers for each of
them, is the table in `crates/aralo-template/src/highlight.rs`
(`outline(body)` and `catalogue()`). It is the core's list, so every shell
offers the same placeholders in the same words, and a test asserts that every
entry parses back as the placeholder it claims to be.

## Dates and times (implemented)

`{{date}}` with no format writes the locale's own date, and `{{time}}` the
locale's own time: `03/09/2026` and `02:05:07 PM` in `en_US`, `09.03.2026` and
`14:05:07` in `de_DE`. With an argument, the argument is a `strftime` pattern.

The locale is the shell's (`Core::set_locale`, which the Mac app fills from
`Locale.current`), or `LC_ALL` / `LC_TIME` / `LANG` for a terminal, or `en_US`.
A single placeholder overrides it with `| locale: fr_FR`.

There is no clock in the format crate: the moment is handed to it, which is
what lets `fixtures/golden/` pin an expansion to the second (ADR-0014).

### Directives

| Directive | Writes | 9 March 2026, 14:05:07, +01:00, `en_US` |
| --- | --- | --- |
| `%Y` `%y` | Year, four digits or two | `2026` `26` |
| `%m` `%d` `%e` | Month, day, day space-padded | `03` `09` ` 9` |
| `%B` `%b` (`%h`) | Month name, long and short | `March` `Mar` |
| `%A` `%a` | Weekday name, long and short | `Monday` `Mon` |
| `%H` `%k` | Hour 00–23, and space-padded | `14` `14` |
| `%I` `%l` | Hour 01–12, and space-padded | `02` ` 2` |
| `%M` `%S` | Minute, second | `05` `07` |
| `%p` `%P` | The locale's `AM`/`PM`, upper and lower | `PM` `pm` |
| `%j` | Day of the year | `068` |
| `%u` `%w` | Weekday, Monday 1–7 and Sunday 0–6 | `1` `1` |
| `%z` `%:z` | Offset from UTC | `+0100` `+01:00` |
| `%x` `%X` `%c` | The locale's date, time, and both | `03/09/2026` `02:05:07 PM` |
| `%F` `%T` `%R` | `%Y-%m-%d`, `%H:%M:%S`, `%H:%M` | `2026-03-09` `14:05:07` `14:05` |
| `%D` `%r` | `%m/%d/%y`, `%I:%M:%S %p` | `03/09/26` `02:05:07 PM` |
| `%n` `%t` `%%` | A newline, a tab, a per cent | |

A flag between the `%` and the letter changes the padding of a number: `%-d`
drops it, `%_d` pads with a space, `%0e` pads with a zero. Outside a
placeholder, `%` is an ordinary character: `%%` and `%n` in body text are the
two characters they look like.

Anything else — `%Z`, `%s`, `%G` — is an error naming the directive, and the
placeholder stays in the document as written. Aralo carries no time-zone
names, only the offset, because a name would mean carrying tzdata.

### Locales

`en_US` `en_GB` `de_DE` `fr_FR` `es_ES` `it_IT` `pt_BR` `nl_NL` `sv_SE`
`da_DK` `nb_NO` `tr_TR` `ja_JP` `zh_CN` `ko_KR`

A tag may be written `de`, `de_DE`, `de-DE` or `de_DE.UTF-8`; case and
separators do not matter. A region Aralo does not carry falls back to the
language, so `de_AT` reads `de_DE`. A language it does not carry reads `en_US`
and says so. The tables are in `crates/aralo-template/src/datetime.rs`; adding
a locale is a table entry and a test.

## Forms (implemented)

`{{field}}` and `{{choice}}` build one form, in the order the placeholders
appear in the body, nested snippets included. **The same name twice is one
field**, and the first one in the body shapes it: its `label:`, its
`default:`, its options.

- `label:` is what to write beside the box. Without one, the name is used.
- `default:` is what the box starts with. A field with no default starts
  empty, and inserts nothing if the user leaves it that way.
- `lines:` is how tall the box is, for an answer with line breaks in it.
  Without it, or with `lines: 1`, the box is one line tall. A body may ask for
  more than 20: the box stops growing there and scrolls, because a panel taller
  than the screen is a panel with no buttons on it. A `lines:` Aralo cannot
  read is left out, with a note, and the box is one line tall. `lines:` on a
  `{{choice}}` is ignored: a drop-down has nothing to type into.
- `options:` is a comma-separated list, trimmed. A drop-down always stands on
  one option: its `default:`, or the first in the list. A `{{choice}}` with no
  options is an error.
- An answer for a name the form does not have is ignored.
- `{{field}}` or `{{choice}}` with no name is an error and stays as written.

## Context (implemented for the clipboard)

`{{clipboard}}` is the clipboard as plain text, fetched by the shell when the
body asks for it and at no other time. If nothing supplied it, the
placeholder stays as written with a note; an empty clipboard is an empty
string, not a missing one.

`{{selection}}`, `{{app}}` and `{{window}}` are in the format and in the
editor's insert menu, and expand to themselves for now. In the MVP they are
available to AI blocks, and only when the snippet declares them in
`ai.context` (PRD P4, D12).

## Nested snippets (implemented)

`{{snippet: reference}}` inlines another snippet. The reference is an id, an
abbreviation or a label, in that order. The nested body is a body: its
placeholders expand, its fields join the outer form, its own
`{{snippet: …}}` is followed.

- **Depth 8** (PRD D7). The ninth stays as written, with an error saying how
  deep the nesting went and where it gave up.
- **A cycle stops at the snippet that closes it.** The reference that would
  reach a snippet already on the way in stays as written, with an error. This
  is reported when the body is read, so the editor shows it without expanding
  anything.
- A reference that names nothing in the library is an error and stays as
  written.

## Cursor stops (implemented)

`{{cursor}}` marks where the caret is left. `{{cursor: select}}` twice selects
what lies between them. Both insert nothing themselves.

- The move happens after the text, the delimiter and any key press, and counts
  what a user would call characters: a family emoji is one, a Return that went
  back as a key press is one.
- A second `{{cursor}}` is ignored, and a single `{{cursor: select}}` selects
  nothing; both get a note.
- A cursor stop at the very end of a body moves nothing.
- **An expansion that moves the caret cannot be undone with Backspace**, since
  Backspace would eat the text around the caret rather than the expansion. A
  body that wants a cursor stop wants a delimiter that stays text, too: Return
  and Tab go back as key presses, and an app that acted on the key has moved
  on.

## Key presses (implemented)

`{{key: tab}}` and `{{key: return}}` press a key rather than type a
character. `{{key: enter}}` is another spelling of Return, because that is
what TextExpander calls it. The PRD does not ask for them; they are what
TextExpander's `%key:…%` macros need to import without loss, which is where
they came from ([import.md](import.md#the-macro-mapping)).

- The plan inserts the text before the key, presses the key, then goes on
  inserting. An app that acts on the key — moving to the next field, sending
  the message — therefore acts on it in the middle of the expansion, which is
  the point: a snippet can fill a form out.
- Tab and Return are the only keys Aralo presses. `{{key: left}}` and the
  rest stay as written, with a note, rather than becoming a character that is
  not that key.
- **An expansion with a key press in it cannot be undone with Backspace**,
  for the same reason a cursor stop cannot: what the app did with the key is
  not text to take back. `undo_delete_count()` reports that by returning
  nothing.

## AI blocks

- **Declared context only.** An AI block sees only the context kinds listed in
  the snippet's `ai.context`. Undeclared context is never requested from the
  shell, not merely never sent (PRD P4).
- **AI output is literal text.** Model output enters the expansion as literal
  text and is never parsed for placeholders (PRD P10). A model cannot cause a
  clipboard read or a nested snippet by writing `{{…}}`.
- **Static text around an AI block is untouched.**
- **`fallback`** is the text used when the policy check or the network fails,
  and it is what an AI block inserts today. The expansion carries a note
  saying the fallback was used, which the preview panel shows.

## Diagnostics

The parser and the evaluator never fail. Anything they cannot do becomes
literal text plus a diagnostic carrying a byte range into the body. The editor
highlights from the same diagnostics, so what the editor shows is what an
expansion does. An **error** means the body does not read the way it was
written; a **note** is advice.

| Diagnostic | Level | Cause | Result |
| --- | --- | --- | --- |
| `Unclosed` | Error | `{{` with no `}}` after it | The rest of the body is literal text |
| `InvalidName` | Error | The name is empty or not `[a-z][a-z0-9_-]*`, for example `{{Date}}` | The whole `{{…}}` is literal text |
| `MalformedOption` | Error | An option without `key:` | That option is dropped; the placeholder stays |
| `MissingName` | Error | `{{field}}`, `{{choice}}` or `{{snippet}}` with nothing after the colon | Stays as written |
| `MissingOptions` | Error | `{{choice}}` with nothing to choose from | Stays as written |
| `BadFormat` | Error | A date or time directive Aralo cannot write | Stays as written |
| `SnippetMissing` | Error | `{{snippet}}` naming nothing in the library | Stays as written |
| `SnippetCycle` | Error | A snippet that reaches itself | That reference stays as written |
| `SnippetTooDeep` | Error | Nesting past 8 | That reference stays as written |
| `UnknownName` | Note | A well-formed name the format does not define | Stays as written |
| `NotImplemented` | Note | A name Aralo will expand in a later release | Stays as written |
| `AiFallback` | Note | An AI block inserted its `fallback:` | The fallback text |
| `UnknownLocale` | Note | A locale Aralo does not carry | The date is written in `en_US` |
| `NothingSupplied` | Note | Nothing supplied a value the body asks for, such as the clipboard | Stays as written |
| `ExtraCursor` | Note | A second cursor stop | Ignored |
| `LoneSelection` | Note | One `{{cursor: select}}` where a pair is needed | Nothing is selected |
| `BadOption` | Note | An option whose value Aralo cannot read, such as `\| lines: plenty` | That option is left out; the rest of the placeholder stands |
| `UnknownKey` | Note | A `{{key}}` naming a key Aralo cannot press | Stays as written |

A problem found twice — a bad format is read once when the body is read and
once when it is written out — is reported once.

## Case transform (implemented)

When a snippet's `case` is `adaptive`, the way the abbreviation was typed
re-cases the whole expansion after rendering, across cursor stops and form
answers alike. See [matching.md](matching.md#adaptive-case-pattern).

## The trailing delimiter (implemented)

For a delimiter-triggered snippet with `keep_delimiter: true`, the delimiter
goes back after the text. Return and Tab go back as key presses, because the
app may act on them: send the message, move to the next cell. Every other
delimiter is appended to the inserted text. See
[expansion-plan.md](expansion-plan.md).

## Golden files

`fixtures/golden/` holds bodies and the text they expand to, in three locales
on a stopped clock, compared byte for byte. It is where a change to any rule
on this page shows up as a diff. See its
[README](../../fixtures/golden/README.md).

## What holds for any body at all

A body is user text: half-written in an editor, pasted from another expander,
nested inside another snippet. `crates/aralo-template/tests/spans.rs` builds
bodies out of the grammar above and out of grammar nobody would write, and
holds the path to three properties.

* Nothing panics. The bridge turns a panic into an app that stops expanding,
  so reading a body and expanding it never do.
* Every range handed out is a range of the body it was read from: the parser's
  and the evaluator's spans are byte ranges on character boundaries, and an
  editor's outline is UTF-16 code units that make a string, never half of an
  emoji's surrogate pair.
* The plan inserts what the expansion rendered, deletes what the engine says
  was typed, and moves the cursor only back over what it inserted itself.

The same file checks that the crate is pure by expanding twice: the same body
and the same context give the same plan and the same diagnostics.
