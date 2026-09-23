# Import and export

Aralo reads snippets out of other expanders and writes its own library out as
one file. Both live in `crates/aralo-import`, because import and export share
one definition of a snippet outside the library folder
([ADR-0013](../adr/0013-import-and-export.md)).

Nothing here is lossy in silence. Every import ends with a report that names
each snippet and what happened to it, and a macro with no Aralo equivalent is
left in the body as the literal text it was, so a search of the library finds
every one. Bodies are searched literally, which is what makes that promise
hold: `aralo search <library> '%delay'` lists them.

## Formats

| Format | Read | Written | What it is |
| --- | :---: | :---: | --- |
| `textexpander` | yes | no | TextExpander's `.textexpander` property list, XML or binary |
| `csv` | yes | yes | Aralo's CSV, and any other comma-separated file |
| `json` | yes | yes | Aralo's document, and a bare array of snippets |
| `yaml` | yes | yes | The same document in YAML |

Aralo does not write TextExpander files. Writing them would claim a fidelity
in the other direction that nothing here measures.

The format is worked out from the file's first bytes (`bplist00`, `<plist`),
then from its extension, and a file that starts with `{` or `[` is read as
JSON. Content wins over the extension: a property list saved as `.xml` is
still a property list. `--format` overrides the lot.

## The macro mapping

TextExpander's macros are converted as follows. A **note** column entry means
the snippet is reported and counts against fidelity.

| TextExpander | Aralo | Note |
| --- | --- | --- |
| `%%` | `%` | — |
| `%|` | `{{cursor}}` | — |
| `%clipboard` | `{{clipboard}}` | — |
| `%snippet:name%` | `{{snippet: name}}` | — |
| `%filltext:name=N%` | `{{field: N}}` | — |
| `%filltext:name=N:default=D%` | `{{field: N \| default: D}}` | — |
| `%fill:N%` (TextExpander 3) | `{{field: N}}` | — |
| `%fillpopup:name=N:default=A%B%C%` | `{{choice: N \| options: A, B, C}}` | — |
| `%m/%d/%Y` and other date runs | `{{date: %m/%d/%Y}}` | — |
| `%H:%M` and other time-only runs | `{{time: %H:%M}}` | — |
| `%key:space%` | a space | — |
| `%key:tab%` | `{{key: tab}}` | — |
| `%key:return%`, `%key:enter%` | `{{key: return}}` | — |
| `%fillarea:name=N%` | `{{field: N \| lines: 4}}` | — |
| `%fillarea:name=N:lines=M%` | `{{field: N \| lines: M}}` | — |
| `%filldate:name=N%` | `{{field: N}}` | approximated: a date picker became a plain field |
| `%key:left%` and every other key | left as written | not converted |
| `%delay:500%` | left as written | not converted |
| `%@+1D%` date arithmetic | left as written | not converted |
| `%fillpart:…%`, `%fillpartend%` | left as written | not converted |
| a script snippet | imported as `type: script` | not converted: Aralo does not run scripts yet |
| a snippet with no plain text | skipped | unreadable: a picture has nothing to import |
| `{{` anywhere in a body | `\{{` | — |

A date run is a sequence of `%` specifiers with ordinary separators between
them, folded into one placeholder: `%B %e, %Y` becomes
`{{date: %B %e, %Y}}`, not three placeholders. A run is a time when every
specifier in it is one of `HIMSpZz`.

Notes are counted once per snippet: a body with four `%delay%` macros reports
the delay once, so counting notes counts snippets.

### What is deliberately not mapped

**The numeric per-snippet and per-group option codes** (`abbreviationMode`,
`expandAfterMode`) are not read. Their meaning is not documented anywhere this
project can check, and a guess would silently change how a snippet expands —
the one kind of loss a report cannot make visible. Imported snippets inherit
Aralo's defaults, which
[matching.md](matching.md) describes: expand at a delimiter, adapt to the
case that was typed, keep the delimiter. Spike S7 is where a real export
settles this.

**Formatting is dropped.** A rich-text snippet imports as its plain text with
a note. Aralo v0 stores `type: rich` but has no rich body yet
([snippet.md](snippet.md)).

### One ambiguity the source format cannot resolve

A pop-up's options follow its head, each ended by `%`, all on one line:
`%fillpopup:name=Size:default=Small%Medium%Large%`. A literal percent sign
later on the same line is therefore read as another option. There is nothing
in the file that distinguishes them. A line break ends the list, which keeps
the damage to one line, and the converted `{{choice}}` is in the report for
the user to check.

## What to do with a body

`--macros` decides how a body's text is read. A reader that knows the file is
Aralo's own overrides it, because nothing else can tell a converted body from
one that was always a template: Aralo's CSV is recognised by its
`keep_delimiter` column, its JSON and YAML by the `format` key.

| Policy | What it does |
| --- | --- |
| `auto` (the default) | Convert when the body plainly carries TextExpander macros; otherwise treat it as literal text |
| `convert` | Convert everywhere, even where `%d` is more likely a format string |
| `literal` | Convert nothing. A `{{` in the body is escaped and stays literal |
| `template` | The body is an Aralo template already; take it as it is |

`auto` asks for an unmistakable macro — `%|`, `%clipboard`, `%key:`,
`%snippet:`, `%fill`, `%delay:`, `%@`, or a date run of two or more
specifiers — and never for a lone percent sign, so "50% off" and
`printf("%d")` are left alone.

## Where imported snippets go

- A group becomes a folder. Nested groups become nested folders, and
  `--into GROUP` puts everything under a folder of your own above them.
- A file is named after the snippet's label, or its first abbreviation, or
  the first line of its body, lower-cased and hyphenated. A name that
  collides gains `-2`, `-3`, and the snippet is reported as renamed. Renaming
  is advisory: the snippet is still clean.
- Names that the library loader skips (a leading `.` or `_`) and characters
  that no file system takes are replaced, and Windows device names
  (`con`, `nul`, `com1`…) are escaped.
- Every imported snippet is given a ULID as it is written, because a snippet
  on disk always has an identity ([snippet.md](snippet.md#id)).
- An abbreviation another snippet already claims is imported anyway and
  reported: Aralo does not silently drop one of them. Only one will expand
  ([matching.md](matching.md#tie-breaks)).

Nothing already in the library is overwritten. A dry run reports what would
happen, including the path each snippet would take, and writes nothing.

## The report

| Outcome | Meaning |
| --- | --- |
| clean | Imported with nothing lost. These are the snippets fidelity counts |
| needs an edit | Imported, and it will expand, but a note says something a human should look at |
| skipped | Not imported |

| Note | Meaning |
| --- | --- |
| not converted | A macro with no Aralo placeholder, left in the body as literal text |
| approximated | Converted, but not exactly; the detail says what changed |
| renamed | The file name was changed to keep it unique or usable (advisory) |
| duplicate abbreviation | Another snippet already claims this abbreviation |
| unreadable | The source said something this importer could not read |
| empty | No body and no abbreviation |
| not written | The file could not be written; the detail is the file system's reason |

**Fidelity is `clean / total`**: the share of the source that imports with
nothing left for a human to edit. An advisory note alone still counts as
clean. An empty source is 1.0.

M3 asks for 95% or better on the corpus in
[`fixtures/import/`](../../fixtures/import/README.md), which
`crates/aralo-import/tests/fidelity.rs` measures on every run. That corpus is
written from these documented formats, so the number measures the converter
against this specification rather than against a library exported from a real
installation; the corpus README says so at more length.

## Export

`aralo export` writes the library, or one group of it, as a document of the
same records an import reads:

```json
{
  "format": 0,
  "name": "My snippets",
  "snippets": [
    {
      "id": "01M2Y1SM00PW4KHBCEJCF5EH24",
      "label": "Best regards",
      "abbr": [";br"],
      "group": ["Work"],
      "tags": [],
      "type": "text",
      "body": "Best regards,\nSam"
    }
  ]
}
```

The JSON and YAML documents share one shape,
[`schemas/export.schema.json`](../../schemas/export.schema.json). The CSV
carries the same fields as columns — `abbr`, `label`, `group`, `tags`,
`body`, `type`, `trigger`, `case`, `word`, `keep_delimiter`, `enabled`,
`id` — with `\n` inside a quoted cell separating several abbreviations or
tags, so nothing needs a second file.

Snippets come out in path order, so two exports of an unchanged library are
the same file. `crates/aralo-import/tests/round_trip.rs` checks that every
written format comes back with the same snippets: same ids, same settings,
same bodies. Only file names may differ, because an import builds them from
the label.

What an export does **not** carry:

- **`_group.yaml`.** A snippet keeps the keys written in its own file, not
  the ones it inherits from its folder. Import a group's snippets and they
  arrive with Aralo's defaults where the group used to speak for them.
- **`aralo.yaml`** beyond the library's name, and nothing from outside the
  folder (settings, history, the search index).
- **A temporary id.** A snippet whose file has no `id` yet is exported
  without one rather than being given an identity it does not have. The
  import gives it a new one.

## From the command line

```sh
aralo import Work.textexpander ~/Aralo            # detect, convert, report
aralo import list.csv ~/Aralo --into Imported     # under one folder
aralo import list.csv ~/Aralo --dry-run           # say what would happen
aralo import list.json ~/Aralo --report json      # the report as JSON
aralo export ~/Aralo snippets.json                # the whole library
aralo export ~/Aralo work.csv --group Work        # one group
aralo export ~/Aralo - --format yaml              # to standard output
```

`aralo import` exits 1 when anything needs an edit or was skipped, 2 when the
source could not be read at all. `aralo export` works out the format from the
file name and asks for `--format` when it cannot.
