# Starter library

The snippets a new Aralo library starts with. `aralo-core` embeds this folder
and copies it into a library that has no snippets yet; it never overwrites a
file. There is no `aralo.yaml` here because every library gets its own ID.

This file has no front matter, so the loader reports it as "not a snippet" and
moves on. The library tests rely on that.
