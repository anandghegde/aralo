# ADR-0015: The format grows to meet the importer, rather than the importer approximating

- Status: Accepted (not yet confirmed by the project owner)
- Date: 2026-09-22

## Context

M3's exit criterion holds a number the importer cannot reach on its own:
*import fidelity 95%*. At the end of M2 the corpus in `fixtures/import/` read
92.3%, and the five snippets that were not clean said exactly what was
missing:

| Snippet | What was lost |
| --- | --- |
| `;notes` | `%fillarea%` became a single-line `{{field}}` |
| `;form` | `%key:tab%` became the tab character it types |
| `;wait` | `%delay:500%` has no placeholder |
| `;stamp` | a script snippet; Aralo does not run scripts |
| `;logo` | a picture: no plain text to import |

The last three are scope Aralo has decided against or deferred, and
[ADR-0013](0013-import-and-export.md) already says what happens to them: the
text stays in the body and the report names it. The first two are different.
They are not features TextExpander has and Aralo rejected; they are features
Aralo's form panel and injector already had, that the format had no words for.
A form panel that draws boxes could draw a taller one. A plan that presses
Return for a kept delimiter could press one in the middle.

That leaves the choice of where to close the gap: in the importer, which could
go on approximating and call the result clean, or in the format.

## Decision

- **The format grows; the importer converts.** Two additions,
  [documented with everything else](../format/placeholders.md):
  `{{field: name | lines: N}}` is a box with room to write in, and
  `{{key: tab}}` / `{{key: return}}` presses a key the app acts on. The
  importer maps `%fillarea:…%` and `%key:tab|return|enter%` onto them and
  leaves no note, because nothing was lost.
- **Fidelity is not raised by relabelling.** An approximation does not become
  clean because the note was deleted. The two snippets count as clean now
  because what they asked for is what they get: the corpus reads 95.4%, and
  `crates/aralo-import/tests/fidelity.rs` raises `FLOOR` from 0.90 to 0.95.
- **Only Tab and Return.** They are the two keys `Step::KeyPress` has, the two
  the injector posts, and the two an app acts on in a way text cannot imitate.
  Every other `%key:…%` stays literal with an `UnknownKey` note, as before: a
  `{{key: left}}` that Aralo turned into nothing, or into a character, would
  be the silent reinterpretation ADR-0013 rules out.
- **A key press costs undo.** `undo_delete_count()` already returns nothing
  when a plan holds a `KeyPress`, because Backspace cannot take back what the
  app did with the key. A body that fills a form in is a body whose expansion
  the user retypes rather than undoes, and the panel and the goldens say so.
- **A box stops growing at 20 lines.** `MAX_FIELD_LINES` clamps what a body
  asks for. A panel taller than the screen is a panel with no buttons on it,
  and a source that says `lines=400` means "a lot", not four hundred.

## Consequences

- The evaluator renders into parts rather than into strings: a rendering is
  segments of text runs and key presses, so a plan can interrupt its typing
  with a key and still count graphemes for the cursor move. `{{cursor}}`
  counts a key as one character.
- A form with a tall box gives Return to the box, so the Mac panel inserts
  with Command and Return and says so on the panel. A form of ordinary boxes
  is unchanged: Return still inserts.
- ADR-0013's list of what an import may lose is shorter by two rows. What is
  left — delays, scripts, pictures, date arithmetic, optional sections — is
  scope, not translation, and each still leaves the source text in the body.
- The floor only goes up. A converter that loses ground loses it on somebody's
  library, so 0.95 is a floor regressions fall through, not a target.
- The same two placeholders are now things a user can write by hand, an editor
  offers in its Insert menu, and the golden suite pins
  (`fixtures/golden/keys.toml`). They are the format's, not the importer's.

## Alternatives considered

- **Drop the note and keep the approximation.** Rejected: it would make the
  number say something untrue, which is the one thing ADR-0013's report exists
  to prevent.
- **Add the corpus snippets that already convert cleanly until the average
  rises.** Rejected for the same reason. `fixtures/import/README.md` states
  the composition so that this is visible if it is ever attempted.
- **A general `{{key: …}}` covering arrows, Escape and function keys.**
  Deferred. The plan can only press what the injector posts and what a test
  can assert; the rest would be a promise the shells do not keep. The
  diagnostic names the two that work rather than failing silently.
- **A `multiline: true` flag instead of `lines: N`.** Rejected: the panel has
  to choose a height anyway, and a number the body writes is one the user can
  tune. It is also what the sources carry (`lines=6`).
- **Support `%delay:…%` too, for a sixth clean snippet.** Rejected for now. A
  delay is a compatibility hack for apps that lose keystrokes, and Aralo's
  answer to those apps is the injection profile — per-app data, not a number
  in a body. Adding it would also need a clamp in the injector before a body
  could ask an app to wait.
