# ADR-0013: One crate for import and export; what an import may lose

- Status: Accepted (not yet confirmed by the project owner)
- Date: 2026-09-21

## Context

M2 has to move a library in and out of Aralo, and its exit criterion is a
number: *import a TextExpander library, edit it and expand from it; import
fidelity ≥ 90% on the corpus*. Two things follow from that sentence. The
importer has to convert a format nobody here can fully verify, and the
project has to be able to say what the number means without overstating it.

The plan puts importers in `aralo-import` and says nothing about where an
export goes. It also leaves open what an importer should do with a macro it
cannot convert, which is the decision that shapes everything a user sees.

## Decision

- **Import and export are one crate.** `aralo-import` owns `SnippetRecord`,
  the one definition of a snippet outside the library folder. Import reads
  it, export writes it, and `export` then `import` is a round trip a test
  checks (`crates/aralo-import/tests/round_trip.rs`). Splitting them would
  mean two definitions of the same thing drifting apart, and an export that
  Aralo cannot read back.
- **Nothing is invented and nothing is dropped in silence.** A macro with no
  Aralo placeholder stays in the body as the literal text it was and the
  snippet gets a note, so the import report lists it and a search of the
  library finds every one. A snippet Aralo cannot run yet (a script) is
  imported and flagged rather than discarded. A `{{` already in a source body
  is escaped to `\{{`, because in Aralo it would open a placeholder.
- **The report is the product.** An import returns an `ImportReport`: every
  snippet, what happened to it, and why. The CLI prints it, the app will show
  it, and `--dry-run` produces it without writing anything. Fidelity is
  derived from it — `clean / total`, where clean means no note beyond an
  advisory rename — rather than being a number the code asserts about itself.
- **TextExpander's numeric option codes are not read.**
  `abbreviationMode` and `expandAfterMode` are not documented anywhere this
  project can check. A guess would silently change how a snippet expands,
  which is the one loss a report cannot make visible. Imported snippets
  inherit Aralo's defaults. Spike S7 settles this against a real export.
- **Aralo does not write TextExpander files.** Exporting to a format whose
  round trip nothing here measures would claim a fidelity the project cannot
  stand behind. The three interchange formats it writes — JSON, YAML, CSV —
  are the ones it also reads.
- **The corpus is synthesized, and says so.** `fixtures/import/` is written
  from the documented formats, because nobody on this project has a
  TextExpander library to export. The harness prints the number on every run
  and its README states the composition and the caveat. The 90% is a floor
  regressions fall through, not a promise about anyone's library.
- **Four dependencies, all already on the licence allow-list.** `plist` for
  the property list (XML and binary in one reader, `default-features = false`
  to keep its optional extras out), `csv` for the tables, `serde_json` and
  `serde_yaml_ng` for the documents — the last two are already in the
  workspace for the library format.

## Consequences

- A user who imports a library with many key presses, delays or scripts sees
  a lower number than the corpus does, and sees exactly which snippets caused
  it. That is the intended trade: an honest report beats a flattering one.
- Unconverted macros sit in bodies as literal text. They will expand as that
  text until the user edits them. The report and the library's search are how
  they are found; a "needs an edit" filter in the editor is the M2 app work
  that makes this comfortable.
- Export carries a snippet's own front matter, not what it inherits from
  `_group.yaml`. Exporting one group and importing it elsewhere therefore
  loses group defaults. Carrying groups would mean a second document type;
  `docs/format/import.md` documents the limitation instead.
- The fidelity number moves when the corpus changes. It is only comparable
  between runs on the same corpus, which is why the composition is written
  down.

## Alternatives considered

- **Refuse what cannot convert.** Rejected: a user who imports 500 snippets
  and gets 480 has lost 20 without being able to find them. Keeping the text
  keeps the choice with the user.
- **A separate `aralo-export` crate.** Rejected: one shared record type is
  the whole reason the round trip is checkable.
- **Guess the numeric option codes from behaviour.** Rejected without a real
  export to test against. A wrong guess is a silent behaviour change.
- **Write TextExpander files too.** Deferred. It needs its own corpus and its
  own measurement, and nobody has asked for it.
