# The library folder

A library is a folder. It can live anywhere: a local folder, iCloud Drive,
Dropbox, a network share, a Git working copy. Aralo does not sync. It behaves
well inside a folder that something else syncs.

The folder is the only source of truth. Everything Aralo keeps outside it is
a cache or a secret, listed [at the end](#what-lives-outside-the-folder).

The reference implementation is `crates/aralo-library` for the folder and
`crates/aralo-snippet` for the files.

## Layout

```text
My Library/                       # any folder
  aralo.yaml                      # format version, library ID
  _group.yaml                     # optional: settings for the whole library
  Support/                        # a group is a folder
    _group.yaml                   # name, colour, icon, enabled, scope, defaults
    refund-enterprise.md          # one snippet per file
    Billing/                      # nested group
      _group.yaml
      late-invoice.md
  assets/                         # reserved: images for rich snippets (v1)
```

| Entry | Meaning |
| --- | --- |
| `aralo.yaml` | The manifest. Marks the folder as a library and carries the format version |
| A folder | A group. Groups nest |
| `_group.yaml` | Optional settings for the folder it sits in, including the root |
| `*.md` | A snippet. See [snippet.md](snippet.md). The extension is matched without regard to ASCII case |
| `assets/` at the root | Reserved for images of rich snippets (v1). Not read as a group |

## What the loader skips

| Entry | Treatment |
| --- | --- |
| A name that starts with `.` | Skipped silently. This covers `.git`, `.DS_Store` and Aralo's own temporary files |
| A name that starts with `_` | Skipped silently. `_group.yaml` is read as the group file, not as a snippet. A folder such as `_drafts/` is a way to park snippets |
| A file without the `.md` extension | Skipped silently |
| A `.md` file without front matter, such as a `README.md` | Skipped, reported as "not a snippet". Not an error |
| A snippet file larger than 1 MiB | Skipped, reported |
| A symbolic link to a folder | Not followed, reported. A symbolic link to a file is read |
| Folders nested more than 32 deep | Not followed, reported |
| A name that is not valid Unicode | Skipped silently |

One bad file never stops a library from loading. Whatever cannot be read
becomes a diagnostic with the file's path, and the rest of the library works.
The only failures that stop a load are an unreadable root folder and a bad
`aralo.yaml`.

## `aralo.yaml`

```yaml
format: 0
id: 01J8ZK3V5Q8W6T9X2N4R7M0ABD
name: My Library
```

Schema: [`schemas/manifest.schema.json`](../../schemas/manifest.schema.json).

| Key | Type | Required | Meaning |
| --- | --- | --- | --- |
| `format` | integer | yes | The library format version. This document describes `0` |
| `id` | string | no | Names this library in the caches Aralo keeps outside the folder. Aralo writes a ULID when it creates a library. This version accepts any string |
| `name` | string | no | Display name |

Unknown keys are preserved.

**A build refuses to open a library whose `format` is newer than it
understands.** An old Aralo therefore never rewrites a library it only half
understands. See the [versioning policy](README.md#versioning-policy).

A folder with no `aralo.yaml` still loads, so you can point Aralo at a folder
of snippet files and try it. Aralo writes the manifest when it creates a
library, and never overwrites one that exists.

## Groups and `_group.yaml`

Every folder under the root is a group. The root is a group too, with no
name. `_group.yaml` is optional, and so is every key in it. A folder without
the file, or with an empty file, is a group named after the folder that
inherits everything.

```yaml
name: Support
colour: "#3478F6"
icon: lifepreserver
enabled: true
scope:
  only: [com.apple.mail, com.tinyspeck.slackmacgap]
defaults:
  trigger: delimiter
  case: adaptive
  word: true
  keep_delimiter: true
  delimiters: " \t\n.,;"
```

Schema: [`schemas/group.schema.json`](../../schemas/group.schema.json).

| Key | Type | Meaning |
| --- | --- | --- |
| `name` | string | Display name. Absent: the folder name |
| `colour` | string | Display colour. Stored as written, not checked. Quote it: an unquoted `#` starts a YAML comment |
| `icon` | string | Display icon name. Stored as written, not checked |
| `enabled` | boolean | `false` switches off the group and everything beneath it |
| `scope` | mapping | The apps the group's snippets work in |
| `defaults` | mapping | Values the group's snippets and sub-groups inherit |

Unknown keys are preserved at the top level and inside `defaults`.

If `_group.yaml` does not parse, the problem is reported and the group
behaves as if the file were absent: it inherits everything from its parent.
A broken group file does not switch its snippets off.

### `defaults`

| Key | Values | Built-in default |
| --- | --- | --- |
| `trigger` | `immediate`, `delimiter` | `delimiter` |
| `case` | `exact`, `ignore`, `adaptive` | `adaptive` |
| `word` | boolean | `true` |
| `keep_delimiter` | boolean | `true` |
| `delimiters` | string | The 23 characters listed in [matching.md](matching.md#default-delimiters) |

Every character of `delimiters` is a delimiter. Use a double-quoted YAML
string so that `\t` and `\n` are read as tab and newline. `delimiters` exists
at group level only. A snippet cannot set its own.

### `scope`

```yaml
scope:
  only: [com.apple.mail]      # these apps and nowhere else
```

```yaml
scope:
  except: [com.apple.Terminal]  # everywhere but these apps
```

```yaml
scope: {}                     # everywhere, whatever the parent says
```

- Entries are bundle IDs on macOS, compared without regard to ASCII case.
  Executable names are intended for Windows, later.
- Set `only` or `except`, never both. A file with both lists non-empty is
  rejected as a conflicting scope.
- `scope` exists at group level only. A snippet has no `scope` key. To scope
  one snippet, put it in a group.
- Unknown keys inside `scope` are accepted on read but are **not** written
  back. This is the one exception to the unknown-keys rule in format v0.

## Inheritance

Each of `trigger`, `case`, `word` and `keep_delimiter` resolves in this
order, and the first level that sets the key wins:

1. the snippet's own front matter
2. `defaults` in the snippet's group
3. `defaults` in each parent group in turn, up to the root
4. the built-in default

`delimiters` resolves the same way from step 2.

`scope` is **replaced, not combined**. A group that sets `scope` uses exactly
that scope, whatever its parents say. A group that does not set it uses its
parent's. The root default is everywhere.

`enabled` is **sticky**. A snippet is enabled only if it and every group
above it are enabled. A sub-group or snippet cannot switch itself back on
inside a disabled group.

A key set to `null` is the same as a key left out.

## Snippet identity

A snippet is identified by the `id` in its front matter, never by its path.

- **No `id`.** The snippet loads and works. It gets a temporary ID that lasts
  until the next load, and the file is reported as missing an ID. Aralo
  writes a permanent one the first time it saves the snippet.
- **Duplicate `id`.** The first file in path order keeps the ID. Every other
  file with that ID is ignored and reported, with the path of the file that
  won. This happens when a snippet file is copied by hand: delete the `id`
  line in the copy.

## What reaches the engine

Every abbreviation of every snippet that is enabled and has `type: text`.
A snippet of another type loads, is reported as a type this build cannot
expand yet, and gets no abbreviations. An abbreviation that the engine
rejects (empty, or longer than 63 characters) is reported, and the rest of
the snippet still works.

## Writes

Aralo writes a file by writing a temporary file in the same folder, flushing
it to disk, and renaming it over the target. A reader, a sync client or a
crash never sees half a file.

The temporary file is named `.<name>.<process id>.aralo-tmp`. It starts with
a dot, so the loader and most sync clients ignore it. If you find one after a
crash it is safe to delete.

What a rewrite changes inside a snippet file is listed in
[snippet.md](snippet.md#what-aralo-changes-when-it-rewrites-a-file).

## Not implemented yet

These parts are planned for milestone M2 and are not in the code today:

- Watching the folder for changes. Today a caller reloads.
- The SQLite index, full-text search and embeddings.
- Conflict-copy detection and three-way merge for synced folders.

## What lives outside the folder

By construction, nothing secret and nothing machine-specific is stored in the
library folder, so it cannot sync by accident (PRD P13).

| What | Where | Notes |
| --- | --- | --- |
| API keys | The system keychain | Never in a file |
| Provider profiles | `~/Library/Application Support/Aralo/` | Names, endpoints, models. No keys |
| Search index, vectors, merge bases | `~/Library/Application Support/Aralo/` | Caches. Safe to delete; rebuilt from the folder |
| Usage statistics | `~/Library/Application Support/Aralo/` | Local only, never synced, not rebuildable |

These locations are from the plan. None of them is written by the code today.
