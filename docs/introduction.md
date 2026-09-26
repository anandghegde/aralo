# Aralo

Aralo is a free, open-source AI text expander. You type a short abbreviation
and it becomes the full text, in any app. Snippets can ask a language model
for part of their text, using your own API key or a model on your own
machine. There is no Aralo account and no Aralo server.

It is a Rust core behind a thin native shell. macOS comes first.

A library is a folder of Markdown files with YAML front matter: one snippet
is one file, one group is one folder. [The library format](format/README.md)
describes it in full.

The privacy promises are properties of the build, each held by a test: see
[the data flow](data-flow.md) for every host Aralo can contact, and
[the threat model](threat-model.md) for the test behind each defence.

This site is built from the `docs/` folder of
[the repository](https://github.com/anandghegde/aralo). Where it and the code
disagree, the repository wins.
