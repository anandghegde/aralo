# The import corpus

What `crates/aralo-import/tests/fidelity.rs` measures the exit criterion
against: M2's *import a TextExpander library, edit it and expand from it;
import fidelity ≥ 90% on the corpus*, raised to 95% by M3.

Every source here has a golden file beside it, `<name>.expected.json`, holding
what the import report said and what ended up in the library. The test
compares them byte for byte, so a change in the converter shows up as a diff
of the text a user would have got. To take a deliberate change:

```sh
ARALO_BLESS=1 cargo test -p aralo-import --test fidelity
```

Then read the diff before committing it.

## What the number means

**Fidelity is the share of the source that imports with nothing left for a
human to edit**: `clean / total`, where a snippet is clean when the import
left no note on it beyond an advisory one (a file that was renamed to keep its
name unique). Anything approximated, anything left in the body as literal
text, anything skipped, counts against it.
[`docs/format/import.md`](../../docs/format/import.md) defines each note.

## What the number does not mean

**These files were written from the documented formats, not exported from a
real installation.** Nobody on this project has a TextExpander library to
export, so the corpus is what the format's documentation and the shape of
ordinary snippets say a library looks like. The percentage therefore measures
the converter against the specification. It does not measure it against the
wild, where an export can carry keys this project has never seen.

The field number is owed by spike S7, which ends with a real export and an
ADR. Until then, treat this as a floor that regressions fall through, not as
a promise about anyone's library.

## What is in it

| Source | Snippets | Clean | What it is for |
| --- | ---: | ---: | --- |
| `textexpander/personal.textexpander` | 22 | 22 | A personal library: addresses, signatures, a cursor, the clipboard, `%%`, and six date and time formats, in three groups |
| `textexpander/work.textexpander` | 20 | 17 | A work library: fill-ins, a pop-up, a nested snippet, two nested groups, and the three things that cannot convert cleanly (below) |
| `csv/spreadsheet.csv` | 8 | 8 | A spreadsheet export: a header in someone else's spelling, a multi-line cell, tags |
| `csv/no-header.csv` | 4 | 4 | A file with no header row, read positionally as abbreviation, body, label |
| `json/other-tool.json` | 6 | 6 | A bare JSON array, as another tool or a hand-written file gives it, including braces that are text |
| `yaml/other-tool.yaml` | 5 | 5 | The same, in YAML |
| **Total** | **65** | **62** | **95.4%** |

The three that are not clean are all in `work.textexpander`, and they are the
three kinds of loss the importer has left:

| Snippet | Note | Why |
| --- | --- | --- |
| `;wait` | not converted | `%delay:500%` has no placeholder; it stays in the body as text |
| `;stamp` | not converted | a script snippet; Aralo does not run scripts yet |
| `;logo` | skipped | a picture: there is no plain text to import |

That is three losses in sixty-five snippets. Two more were losses until M3
gave the format what they needed: `;notes` lands in a box with room to write
in (`{{field: Notes \| lines: 4}}`) and `;form` presses a real Tab between the
two fields it fills (`{{key: tab}}`). A library that leans harder on delays
and scripts would score lower, and the report says which snippets they are
every time, so the user can see what they have rather than a percentage.

Each of those macros is also covered on its own by the unit tests in
`crates/aralo-import/src/macros.rs`; the corpus is here to weigh them as an
ordinary library would, not to cover them.
