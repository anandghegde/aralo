# Aralo library format

An Aralo library is a folder of plain files. One snippet is one Markdown file
with YAML front matter. One group is one folder. There is no database in the
folder and nothing in it is binary, so any editor, any diff tool and any sync
service can handle it.

This directory is the specification. It is written for two readers: people who
edit snippet files by hand, and people who write other tools or shells that
read the same folder.

**Current version: format v0 (pre-release).**

## Documents

| Document | Covers |
| --- | --- |
| [library.md](library.md) | Folder layout, `aralo.yaml`, groups and `_group.yaml`, inheritance, what is skipped, what lives outside the folder |
| [snippet.md](snippet.md) | The snippet file: every front-matter key, the body rules, what Aralo changes when it rewrites a file |
| [placeholders.md](placeholders.md) | The `{{…}}` grammar for snippet bodies and the MVP placeholders |
| [matching.md](matching.md) | How typed keys become a match: triggers, delimiters, whole word, case, scope, tie-breaks, undo, resets |
| [expansion-plan.md](expansion-plan.md) | The `ExpansionPlan` a shell's injector executes |
| [import.md](import.md) | Reading other expanders' files, the macro mapping, what fidelity means, and the interchange formats Aralo writes |

## Schemas

Machine-readable JSON Schemas (draft 2020-12) live in [`schemas/`](../../schemas/):

| Schema | Describes |
| --- | --- |
| [`snippet.schema.json`](../../schemas/snippet.schema.json) | The YAML front matter of a snippet file |
| [`group.schema.json`](../../schemas/group.schema.json) | `_group.yaml` |
| [`manifest.schema.json`](../../schemas/manifest.schema.json) | `aralo.yaml` |
| [`expansion-plan.schema.json`](../../schemas/expansion-plan.schema.json) | `ExpansionPlan` as JSON |
| [`compat.schema.json`](../../schemas/compat.schema.json) | The app compatibility table, `data/compat/apps.toml` ([architecture](../architecture.md#the-compatibility-table)) |
| [`export.schema.json`](../../schemas/export.schema.json) | The interchange document `aralo export` writes ([import.md](import.md)) |

The library files are YAML and the compatibility table is TOML; the schemas
apply to the data after parsing. The `$id` values use the placeholder domain
`aralo.invalid` until the project has a domain. They do not resolve.

Schemas for the policy file and the provider quirks table are planned (plan
section 5.2) and not written yet.

## Reference implementation

| Part | Crate |
| --- | --- |
| Parsing and writing `aralo.yaml`, `_group.yaml` and snippet files | `crates/aralo-snippet` |
| Matching | `crates/aralo-engine` |
| Placeholder grammar, static rendering, `ExpansionPlan` | `crates/aralo-template` |
| Walking the folder, resolving inheritance, atomic writes | `crates/aralo-library` |
| Reading other expanders' files, the import report, the interchange formats | `crates/aralo-import` |

Where this specification and the code disagree, that is a bug in one of them.
Please open an issue. Until format v1, the code is what a library is tested
against.

## Versioning policy

`aralo.yaml` carries the format version as the integer `format`.

1. **Format v0 is pre-release.** It may change without a migration. Do not
   build long-lived tooling on v0 without expecting to follow changes.
2. **From v1, changes within a major version are additive only.** New optional
   keys, new enum values in places documented as open, new placeholders. No
   key is removed or changes meaning within a major version.
3. **Unknown keys are preserved.** Every file type keeps the keys it does not
   understand and writes them back unchanged, so a newer Aralo and an older
   one can share a folder. The one exception in v0 is inside `scope`; see
   [library.md](library.md#scope).
4. **A build refuses to open a library that is newer than it understands.** If
   `format` in `aralo.yaml` is greater than the build's format version, the
   library does not open. An old Aralo therefore never rewrites a library it
   only half understands.
5. **A new major version comes with a migration** and a documented way back
   where one is possible.

Schemas are versioned with the format: the `v0` in each `$id` is the format
version.

## What is implemented today

The project is pre-alpha. This table is the honest state of format v0.

| Part | State |
| --- | --- |
| Snippet, group and manifest files: parse, write, unknown-key preservation, format refusal | Implemented in `aralo-snippet`, with tests |
| Matching rules | Implemented in `aralo-engine`, with property tests against a naive reference matcher |
| Placeholder parsing (`{{…}}`, options, escapes, diagnostics) | Implemented in `aralo-template`, without `{{if}}` blocks |
| Placeholder evaluation (dates, clipboard, fields, nested snippets, AI) | Specified. Not implemented; the evaluator lands in milestone M3. Today a placeholder is inserted as its source text |
| Folder walking, inheritance, `enabled`, skipped names, duplicate and missing IDs, atomic writes | Implemented in `aralo-library`, with tests |
| Import (TextExpander, CSV, JSON, YAML), the import report, export to JSON, YAML and CSV | Implemented in `aralo-import`, with a golden corpus and a fidelity harness ([import.md](import.md)) |
| Folder watching, the search index | Implemented in `aralo-library`, with tests. Not yet wired into `aralo-core` |
| Conflict merging | Specified in the plan. Not implemented; planned for M2 |
| `type` values other than `text` | Accepted, stored and reported. Such a snippet gets no abbreviations yet |
