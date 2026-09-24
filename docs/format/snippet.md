# The snippet file

One snippet is one UTF-8 Markdown file with the extension `.md`. It has two
parts: YAML front matter between two `---` lines, and a body.

```markdown
---
id: 01J8ZK3V5Q8W6T9X2N4R7M0ABC
label: Enterprise refund reply
abbr: [";refund", ";rf"]
type: text
trigger: delimiter
case: adaptive
word: true
tags: [support, billing]
ai:
  context: [fillins, selection]
  profile: default
---
Hi {{field:name | label: Customer name}},

Your refund of {{field:amount}} was approved on {{date:%d %B %Y}}.

{{cursor}}
```

The reference implementation is `crates/aralo-snippet/src/snippet.rs`. The
front matter is described by
[`schemas/snippet.schema.json`](../../schemas/snippet.schema.json).

The file name is for people. Aralo identifies a snippet by its `id`, so a
file can be renamed or moved to another group without losing its identity.

## Structure

1. A byte-order mark at the start of the file is ignored.
2. The first line must be exactly `---`. A trailing carriage return is
   tolerated, so CRLF files work. A file that starts with anything else is
   rejected as having no front matter.
3. The front matter runs to the next line that is exactly `---`. If there is
   none, the file is rejected as unclosed.
4. Everything after that line is the body.

An empty front-matter block is valid. Every key then takes its default:

```markdown
---
---
Just a body.
```

## Front-matter keys

Every key is optional. For `trigger`, `case`, `word` and `keep_delimiter`, a
key that is absent or `null` means "inherit from the group"; see
[library.md](library.md#inheritance).

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `id` | ULID string | none | The snippet's identity. Hand-written files may leave it out; Aralo assigns one on the first save |
| `label` | string | empty | Name shown in lists and search |
| `abbr` | string or list of strings | none | The abbreviations that expand this snippet. With none, the snippet is reachable through search only |
| `type` | `text`, `rich`, `command`, `prompt`, `script` | `text` | The kind of snippet. `text` is inserted. `command` is a command on selected text: its body is the instruction, and it is offered in the command panel rather than expanded (see [below](#type-command)). The others load with a warning and do nothing yet |
| `trigger` | `immediate`, `delimiter` | inherit, then `delimiter` | When the abbreviation fires. See [matching.md](matching.md#triggers) |
| `case` | `exact`, `ignore`, `adaptive` | inherit, then `adaptive` | How case is matched and applied. See [matching.md](matching.md#case) |
| `word` | boolean | inherit, then `true` | Whole-word rule. See [matching.md](matching.md#whole-word) |
| `keep_delimiter` | boolean | inherit, then `true` | Re-insert the triggering delimiter after the expansion. Only read for `trigger: delimiter` |
| `enabled` | boolean | enabled | `false` switches the snippet off |
| `tags` | list of strings | none | Free-form tags for search and filtering |
| `ai` | mapping | none | What an AI block may see, and which provider profile runs it |

### `id`

A ULID: 26 characters of Crockford base32, which is the digits and the
letters without I, L, O and U. Aralo writes upper case. Lower case is
accepted on read, and surrounding white space is trimmed.

Two snippets must not share an ID. When you copy a snippet file by hand,
delete the `id` line in the copy.

### `abbr`

Either form is accepted on read:

```yaml
abbr: ";sig"
abbr: [";sig", ";signature"]
```

Aralo always writes a list.

An abbreviation must be between 1 and 63 characters long. The engine rejects
one that is empty or longer, reports it, and loads the rest of the library.

Quote abbreviations. Many useful ones start with a character that YAML treats
specially, such as `;`, `:`, `#`, `@`, `!`, `*` or `&`. Unquoted `#sig` is a
comment and unquoted `yes` is a boolean.

### `ai`

| Key | Type | Meaning |
| --- | --- | --- |
| `context` | list of strings | The context kinds this snippet declares. Nothing else may be requested from the shell or sent to a model. The plan names `fillins`, `selection`, `clipboard`, `app` and `window`. This version stores the strings without checking them |
| `profile` | string | Name of the provider profile to use. Absent: the profile chosen in settings. Profiles live outside the library folder and never hold a key in a file |
| `model` | string | The model to ask. Absent: the profile's default model |

Unknown keys inside `ai` are preserved.

### Unknown keys

Keys that this version does not know are kept and written back unchanged, at
the top level and inside `ai`. That lets a newer Aralo and an older one share
a folder, and lets other tools keep their own metadata in the front matter.

## The body

The body is everything after the closing `---` line.

- **One final line ending is not part of the body.** Exactly one trailing
  `\n` or `\r\n` is removed on read, and one `\n` is added on write. A body
  that must end with a blank line has two line endings in the file.
- **Everything else is kept byte for byte.** Line endings inside the body are
  not normalised, so a CRLF body stays CRLF.
- **A `---` line inside the body is body text.** Only the first closing
  `---` ends the front matter.
- The body is a template. `{{…}}` placeholders are described in
  [placeholders.md](placeholders.md). To write a literal `{{`, write `\{{`.

For `type: text` the body is inserted as plain text. It is not rendered as
Markdown. The `.md` extension is there so that editors and Git hosts show the
file sensibly.

### `type: command`

A command runs on the text selected in another app, and its answer replaces
that text. The body is the instruction the model is given, in plain words. It
is not a template: `{{…}}` is sent as written. The selection is the only
context a command sends, whatever `ai.context` says. `ai.profile` and
`ai.model` choose who answers.

```markdown
---
label: Make it sound like a pirate
type: command
ai:
  model: gpt-4o-mini
---
Rewrite the text the way a pirate would say it. Keep its meaning.
```

A command has nothing to insert, so its abbreviations expand nothing. Aralo
ships seven built-in commands (`data/commands/`). A library command with the
same `id` as a built-in one replaces it in the list.

## What Aralo changes when it rewrites a file

When Aralo saves a snippet it writes the front matter again from the parsed
data. The body is not touched beyond the final line ending. Expect these
changes to a hand-written file:

| Change | Detail |
| --- | --- |
| Key order | Known keys are written in the order of the table above, then unknown keys in sorted order |
| `abbr` | Always a list |
| Defaults dropped | `type: text` and an empty `label` are not written |
| Inherited keys dropped | A `null` value for an inherited key is not written |
| Formatting | Flow lists such as `[a, b]` may become block lists. Quoting may change |
| **Comments** | **YAML comments in the front matter are lost** |

An empty `abbr` or `tags` list is not written either.

The loss of comments is a known limitation of format v0. Keep a note in an
unknown key such as `note:`, which is preserved.

Writes are atomic; see [library.md](library.md#writes).

## Errors

A file that cannot be read as a snippet is reported with its path and
skipped. It never stops the rest of the library from loading.

| Problem | Cause | Treatment |
| --- | --- | --- |
| Missing front matter | The first line is not `---` | Reported as "not a snippet". This is how a `README.md` in the folder is handled. Not an error |
| Unclosed front matter | No closing `---` line | Reported as invalid, skipped |
| YAML error | The front matter is not valid YAML, a key has the wrong type, or `id` is not a ULID | Reported as invalid, skipped |
| Larger than 1 MiB | | Reported, skipped |

Other conditions, such as a missing or duplicate `id`, are covered in
[library.md](library.md#snippet-identity).

## Minimal examples

Reachable by abbreviation, everything else inherited:

```markdown
---
abbr: ";sig"
---
Kind regards,
The support team
```

Expands the moment the last character is typed, in the middle of a word if
need be, exactly as typed:

```markdown
---
label: Arrow
abbr: "->>"
trigger: immediate
case: exact
word: false
---
→
```
