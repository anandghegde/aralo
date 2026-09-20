# Placeholders

A snippet body is a template. Text is inserted as written. `{{…}}`
placeholders are replaced when the snippet expands.

The reference implementation is `crates/aralo-template`.

**Status.** The parser is implemented and tested. The evaluator is not: it
lands in milestone M3. Until then a placeholder is inserted as its own source
text, so `{{clipboard}}` expands to the literal characters `{{clipboard}}`.
The sections below mark what is implemented and what is specified.

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
`if`, `else` and `end`.

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

## The parser never fails

Input the parser cannot read becomes literal text plus a diagnostic. Each
diagnostic carries a byte range into the body. The editor is meant to
highlight from the same diagnostics, so what the editor shows is what an
expansion does.

| Diagnostic | Cause | Result |
| --- | --- | --- |
| `Unclosed` | `{{` with no `}}` after it | The rest of the body is literal text |
| `InvalidName` | The name is empty or does not match `[a-z][a-z0-9_-]*`, for example `{{Date}}` or `{{ }}` | The whole `{{…}}` is literal text |
| `MalformedOption` | An option without `key:` | That option is dropped. The placeholder stays |

An unknown but well-formed name, such as `{{nonsense}}`, is a valid
placeholder to the parser. What the evaluator does with it is not decided
yet.

## MVP placeholders (specified, not implemented)

These are the placeholders planned for the first release. The forms are from
the plan, section 6.1. Details may change before format v1.

| Placeholder | MVP behaviour | PRD |
| --- | --- | --- |
| `{{date: format \| locale: de}}`, `{{time: format}}` | The current date or time in a custom format and locale. Date arithmetic such as `\| add: 3d` is planned for v1 | D1, D2 |
| `{{clipboard}}` | The current clipboard as plain text. History slots are planned for v1 | D3 |
| `{{field: name \| default: x}}` | A single-line form field. The same name used twice resolves to one answer | D4 |
| `{{choice: name \| options: a, b, c}}` | A drop-down | D4 |
| `{{snippet: name-or-id}}` | Another snippet, inlined. Depth limit 8. Cycles are reported at save time | D7 |
| `{{cursor}}` | Where the cursor is left after the expansion. A pair of `{{cursor: select}}` marks a selection | E5 |
| `{{ai: prompt \| fallback: text \| model: x}}` | An AI block. See below | D15, A2 |
| `{{selection}}`, `{{app}}`, `{{window}}` | In the MVP, available to AI blocks only, and only when the snippet declares them in `ai.context`. General use is planned for v1 | P4, D12 |

In this table `\|` is only how a pipe is written inside a Markdown table
cell. In a snippet body write a plain `|`.

## Rules for AI blocks (specified)

- **Declared context only.** An AI block sees only the context kinds listed
  in the snippet's `ai.context`. Undeclared context is never requested from
  the shell, not merely never sent (PRD P4).
- **AI output is literal text.** Model output enters the expansion as literal
  text and is never parsed for placeholders (PRD P10). A model cannot cause a
  clipboard read or a nested snippet by writing `{{…}}`.
- **Static text around an AI block is untouched.**
- **`fallback`** is the text used when the policy check or the network
  fails. The preview panel says so when it happens.

## Case transform (implemented)

When a snippet's `case` is `adaptive`, the way the abbreviation was typed
re-cases the whole expansion after rendering. See
[matching.md](matching.md#adaptive-case-pattern).

## The trailing delimiter (implemented)

For a delimiter-triggered snippet with `keep_delimiter: true`, the delimiter
goes back after the text. Return and Tab go back as key presses, because the
app may act on them: send the message, move to the next cell. Every other
delimiter is appended to the inserted text. See
[expansion-plan.md](expansion-plan.md).
