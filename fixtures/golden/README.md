# Golden expansions

What the evaluator does, pinned to the byte. Each `*.toml` here holds the
inputs of a set of cases; the `*.expected.txt` beside it holds what came out,
written by the test and compared byte for byte:

```sh
cargo test -p aralo-core --test golden        # check
ARALO_BLESS=1 cargo test -p aralo-core --test golden   # take a wanted change
```

Every case runs three times, in `en_US`, `de_DE` and `ja_JP`, on a clock
stopped at **Monday 9 March 2026, 14:05:07, +01:00**. A locale that writes its
months in another language and one that does not use Latin letters at all are
there so that a change to the date tables cannot pass unnoticed; the stopped
clock is why a golden written today still holds tomorrow (ADR-0014).

## A case

```toml
about = "what this file is for"       # goes at the top of the transcript

[[case]]
name = "what the case shows"          # also what a failure names
body = "Hi {{field: who}}, {{date}}"  # the snippet under test, labelled Case
abbr = ";x"                           # optional; only a `typed` case needs another
typed = ";x "                         # optional: type it instead of picking it
answers = { who = "Dana" }            # optional: what the user filled in
clipboard = "PO-8841"                 # optional: what the shell fetched
cancel = true                         # optional: the user presses Escape
nested = [{ label = "Greeting", body = "Hi there" }]   # optional: other snippets
```

The snippet under test is labelled `Case`, so a body that writes
`{{snippet: Case}}` reaches itself. Without `typed`, the case is inserted the
way the palette inserts it: nothing typed, so nothing to delete and no
delimiter to put back. With `typed`, the keys go through the engine, which is
how a case about deleting the abbreviation, re-casing the expansion or putting
a delimiter back is written.

## A transcript

```text
=== the cursor lands where the body asks
--- en_US
field: "Dear |,\nthanks "
preview: "Dear ,\nthanks"
undo: not by backspace
note: "..."
```

`field` is the whole text field afterwards, with `|` where the caret ended up
and `[...]` around a selection. `preview` is what an editor shows beside the
body, which the test also checks is the expansion an empty form would produce,
byte for byte (PRD L10). `undo` is whether Backspace can take the expansion
back. The `note` and `error` lines are what Aralo has to say about the body,
in the words a shell shows the user.

Values are quoted so that a space at the end of an expansion can be seen, and
line breaks and tabs are written `\n` and `\t` so one expansion stays on one
line.
