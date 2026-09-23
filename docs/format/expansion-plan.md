# The expansion plan

An `ExpansionPlan` is what the core hands a shell for one expansion: a list
of steps that the shell's injector runs in order. The plan says **what**
reaches the document. **How** it gets there, typing or paste, comes with the
plan as the front app's injection profile, a row of
[the compatibility table](../architecture.md#the-compatibility-table). The
shell follows both and decides nothing.

The reference implementation is `crates/aralo-template/src/plan.rs`. The JSON
form is described by
[`schemas/expansion-plan.schema.json`](../../schemas/expansion-plan.schema.json).

## Steps

| Step | Fields | Meaning | Produced today |
| --- | --- | --- | --- |
| `Delete` | `count` | Remove `count` characters before the cursor: the abbreviation that already reached the document. Backspace `count` times, or select and delete once where the profile says `delete = "select"` | Yes |
| `InsertText` | `text` | Insert plain text. May contain newlines | Yes |
| `KeyPress` | `key`: `Return` or `Tab` | Press a key the app should see as a key, not as text | Yes |
| `InsertRich` | `html`, `plain` | Insert rich text, with the plain form for apps that refuse it | No. v1 |
| `Delay` | `millis` | Wait. For per-app compatibility rules | No |
| `MoveCursor` | `graphemes`, `select` | Move the cursor left by `graphemes` user-perceived characters, selecting on the way if `select` | Yes, for `{{cursor}}` |

All numbers are unsigned 32-bit integers.

## What a plan looks like

Every plan is built by `plan()` in `crates/aralo-template/src/render.rs`, from
a body that has already been expanded. Its steps come in this order:

1. `Delete { count }`, when the engine's `delete_count` is greater than zero.
2. The expanded body — dates, answers, nested snippets and all, with the
   adaptive case transform applied. It goes in as one `InsertText` per run of
   text, with a `KeyPress` wherever a `{{key: tab}}` or `{{key: return}}`
   interrupts the typing, because the app has to see those as keys rather
   than as characters. A body with no key in it is one `InsertText`, and an
   empty run is no step at all.
3. `KeyPress { key }`, when the delimiter that triggered the match was Return
   or Tab and the snippet keeps its delimiter.
4. `MoveCursor`, when the body has a `{{cursor}}` stop with anything after it:
   left over everything the plan put in after the stop, the key press
   included.
5. A second `MoveCursor` with `select: true`, for the text between a pair of
   `{{cursor: select}}`.

Any other kept delimiter is appended to the text of step 2, so that a cursor
stop lands where the body put it. Return and Tab go back as key presses
because the user pressed a key the app may act on: send the message, move to
the next cell. Text would not do that — but an app that acted on the key has
also moved on, and a `MoveCursor` after it no longer means anything useful. A
body that wants a cursor stop wants a delimiter that stays text.

Typing `;br` and a space, with the body `Best regards,`:

```json
{
  "steps": [
    { "type": "delete", "count": 3 },
    { "type": "insert_text", "text": "Best regards, " }
  ]
}
```

The same with Return instead of the space:

```json
{
  "steps": [
    { "type": "delete", "count": 3 },
    { "type": "insert_text", "text": "Best regards," },
    { "type": "key_press", "key": "return" }
  ]
}
```

A body that fills a form out, `{{field: Name}}{{key: tab}}{{field: Email}}`,
with both boxes answered:

```json
{
  "steps": [
    { "type": "insert_text", "text": "Dana" },
    { "type": "key_press", "key": "tab" },
    { "type": "insert_text", "text": "dana@acme.example" }
  ]
}
```

The delimiter itself is not counted in `delete`: the shell consumed that key,
so it never reached the document. See [matching.md](matching.md#the-verdict).

## Undo

A plan reports how many Backspaces remove what it inserted
(`undo_delete_count`).

- The count is one per **user-perceived character** (extended grapheme
  cluster) of every `InsertText`, because that is the unit text fields delete
  by. Counting code points would over-delete after an emoji and eat the
  user's own text.
- It is **absent** when the plan contains an `InsertRich`, a `KeyPress` or a
  `MoveCursor`. Backspace cannot be trusted to remove rich text, or a Return
  or Tab the app may have acted on, and after a cursor move it would eat the
  text around the caret rather than the expansion. The shell then does not
  offer undo for that expansion.
- For the same reason the shell does not offer undo, whatever the count,
  when the profile made it type a line break of an `InsertText` as a Return
  key (`insert = "type"`).
- `Delete` and `Delay` do not change the count.

When the count is present, the shell reports the finished expansion with
`expansion_done`, and the next Backspace or undo shortcut reverses it. See
[matching.md](matching.md#undo).

## Rules for a shell

1. Run the steps in order, the way the injection profile says.
2. Swallow the key that completed the match when the action says `consume`.
3. Tag every synthetic event so that the shell's own key capture ignores it.
4. The plan is the whole instruction. A shell must not look at a snippet body
   or decide what to expand.

## Encodings

**Across the bridge.** On macOS the plan crosses the UniFFI bridge as the
enum `PlanStep` in `crates/aralo-ffi`, inside `KeyAction.Expand { steps, … }`
or, when the body has a form or needs context first, at the end of an
expansion session: `KeyAction.StartSession` hands over an `ExpansionSession`,
and the plan arrives as its `SessionAction.Expand`. It is not serialised to
JSON.

**JSON.** For tools, tests and future shells, the JSON form is an object with
one key, `steps`. Each step is an object tagged by `type`. Variant and field
names are in snake_case, and keys are lower case:

| `type` | Other keys |
| --- | --- |
| `delete` | `count` |
| `insert_text` | `text` |
| `insert_rich` | `html`, `plain` |
| `key_press` | `key`: `return` or `tab` |
| `delay` | `millis` |
| `move_cursor` | `graphemes`, `select` |

The Rust types do not derive a serialiser yet, so nothing in the repository
reads or writes this JSON today. The schema fixes the form in advance so that
the first code to need it has one to follow.
