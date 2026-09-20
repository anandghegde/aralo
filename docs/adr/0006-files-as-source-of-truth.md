# ADR-0006: The library folder is the only source of truth

- Status: Accepted
- Date: 2026-09-20

## Context

Aralo is free and open source with no accounts and no Aralo servers, so there
is no backend to hold a user's snippets. The PRD asks for one readable file
per snippet, with SQLite and vectors as caches. Users keep their library where
they like: iCloud Drive, Dropbox, a NAS, a Git repository.

## Decision

**The library folder is the only source of truth.**

- One snippet per file: YAML front matter plus a body. A group is a folder
  with a `_group.yaml` file. `aralo.yaml` at the root holds the format version
  and library ID.
- The file name is a readable slug. The `id` is a ULID that survives renames
  and moves.
- Unknown front-matter keys are preserved on save, so a newer Aralo and an
  older one can share a folder.
- Writes are atomic: write a temp file in the same folder, fsync, rename. The
  watcher ignores Aralo's own writes by content hash.
- `schemas/` holds JSON Schemas for the file formats. They are versioned with
  the format and validated in CI.

**Everything else is a disposable cache.** The SQLite index (with FTS5), the
embedding vectors and the merge bases live in
`~/Library/Application Support/Aralo/` and can be deleted at any time. They are
rebuilt from the files. Startup compares modification times and hashes against
the index and reparses only what changed.

**Aralo does not sync.** It behaves well inside a folder that something else
syncs.

- After each successful save or load, Aralo stores that version as the
  snippet's merge base, keyed by snippet ID.
- The watcher recognises conflict copies by each sync provider's naming
  pattern.
- A conflict pair gets a three-way merge: front matter key by key, body line
  by line. A clean merge is written and the conflict copy goes to the Trash. A
  failed merge keeps both files and opens a resolver.

## Consequences

- A user can read, edit, diff, back up and version their library with
  ordinary tools, with or without Aralo installed.
- Deleting the cache folder is always safe. The index is rebuilt from the
  files.
- One exception: the `stats` table (local expansion counts and last-used
  times) cannot be rebuilt from files. It is local only and never synced.
- API keys, provider profiles, statistics and merge bases are outside the
  library folder by construction, so they cannot sync by accident (P13).
- A cold rebuild of 10,000 snippets is a background job. Expansion works as
  soon as abbreviations are loaded, before bodies are indexed.
- Aralo must tolerate whatever a sync tool does to the folder: partial
  writes, conflict copies and files that change under an open editor.

## Alternatives rejected

- **A database as the primary store.** It would not be readable or mergeable
  by other tools, and the PRD specifies one readable file per snippet.
- **Aralo's own sync service.** Aralo runs no servers. Folder sync is left to
  the tool the user already trusts.
