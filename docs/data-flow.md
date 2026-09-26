# Data flow

What Aralo reads, where it keeps it, and where it sends it. The last part is
the complete list of hosts the app can contact. It is also an allow-list:
`crates/aralo-cli/tests/hosts.rs` reads the host tables on this page and fails
when the `aralo` binary, the Mac app's Swift sources or the shipped data name
a host that is not in one of them (PRD P7).

The design behind each promise is in [architecture.md](architecture.md#privacy-invariants).
What each promise defends against, and the test that proves it, is in
[threat-model.md](threat-model.md).

## In one picture

```
             keys                      plan (text to insert)
  event tap ───────▶ aralo-engine ─────────────────────────────▶ injector ──▶ front app
  (AraloKit)         64 characters,                              (pasteboard saved
                     memory only                                  and restored)
                          ▲
                          │ abbreviations (a snapshot)
                          │
  library folder ──▶ aralo-library ──▶ index (SQLite, state folder, never synced)
  (the user's,         loader, watcher,
   may be synced)      merge
                          │
                          ▼
  form answers,     aralo-core session ──▶ aralo-ai gateway ──▶ network guard ──▶ the profile's endpoint
  clipboard,        only declared kinds    policy, framing,     the only HTTP      (the user's own,
  selection,        are asked for          manifest             client             or 127.0.0.1)
  app, window                                   ▲
                                                │ key, for the Authorization header only
                                          login keychain
```

## What Aralo reads, and where it goes

| Data | Read when | Kept where | Leaves the Mac |
| --- | --- | --- | --- |
| Keystrokes | Every key-down the event tap sees, while Aralo is on, not paused, and the front app is not excluded | A ring of 64 characters in `aralo-engine`, in memory. Zeroed on every reset, after every match and when the engine is dropped. The Swift tap keeps no copy | Never. The engine has no I/O, clock, network or logging dependency |
| Snippets | At launch, and when the watcher reports a change | The library folder, as plain files. The user chooses the folder, and may put it in a synced one | Only through the user's own sync client, an export the user asks for, or a model request that declares the text (a command's selection, an editor action's text) |
| The index | Built from the library | A SQLite file per library in the state folder: a copy of each snippet, expansion counts and last-used times, merge bases, and the vectors search by meaning uses | Never. It is outside the library, so a sync client never sees it |
| Clipboard | When a snippet's body has `{{clipboard}}`, or an AI snippet declares `clipboard` | Not kept | Only in a model request whose snippet declared it |
| Pasteboard, while pasting | When the injector inserts by paste | Saved in memory for the paste, then put back. What Aralo puts on it is marked transient and concealed, so clipboard managers skip it | Never |
| Selection | When the user runs a command on selected text, or an AI snippet declares `selection`. Never from a password field, never while secure input is on | Not kept | Only in that command's or snippet's model request |
| Front app | On every app switch: its bundle ID picks the exclusion list and the injection profile | In memory | The app's name, and its window's title, only when an AI snippet declares `app` or `window` |
| Form answers | When the user fills a snippet's form | Not kept | Only when an AI snippet declares `fillins` |
| A model request | When the user asks for one: a command, an AI block, an editor action, Test connection, the probe, the model list, the conformance suite | Not kept | To the profile's endpoint, through the network guard. The system prompt, the instruction and each declared kind framed as data. The panel shows the kind and size of each item that went |
| A model's answer | As it streams | In memory, in the panel, until the user inserts, edits or discards it | Never. It is inserted as literal text and never read for placeholders |
| API keys | When a request to a profile with a key is sent | The login keychain, service `Aralo AI key`. `profiles.toml` holds a `key_ref`, never the key | Only in the `Authorization` header (or the adapter's key header) of a request to that profile's own address. The client follows no redirect |
| AI settings | At launch, and when changed | `profiles.toml` in the state folder: the switch, local-only mode, the profiles, the probe's findings | Never |
| Token usage | When a stream reports it | Counters in memory, per feature and profile | Never. There is no upload code |
| The embedding model | When AI is on and search by meaning is used | Inside the app bundle. Fetched at build time, never at runtime | Never. `aralo-embed` has no networking crate in its dependencies |
| Conformance reports | When the user runs `aralo conformance --report` | Where the user says | Only where the user takes them. A report names the endpoint by host and port, says whether a key was sent, and redacts anything shaped like a key |
| Conflict copies | When a sync client leaves one | Merged into the original, then moved to the Trash | Never |
| Logs | Aralo writes none today | | |
| Crash reports | Written by macOS, not by Aralo, in `~/Library/Logs/DiagnosticReports` | | Only if the user attaches one to an issue |

## The folders

| Folder | Holds | Default |
| --- | --- | --- |
| The library folder | Snippet files, `_group.yaml` files and `aralo.yaml`. Nothing else Aralo writes goes here | `~/Aralo`, or a folder the user chooses; `ARALO_LIBRARY` overrides it |
| The state folder | `profiles.toml`, the index, and conflict copies set aside by a shell with no Trash | `~/Library/Application Support/Aralo`; `ARALO_STATE` overrides it |
| The login keychain | API keys, one item per profile that has one | Service `Aralo AI key` |
| User defaults | Whether onboarding was finished | The app's own domain, `app.aralo.Aralo` |

## Every host Aralo can contact

Aralo has no servers of its own, so there is nothing of Aralo's to contact.
The app sends a request only to an address the user saved in an AI profile,
and only while the AI switch is on, with one exception: a release checks
GitHub for updates (below). In local-only mode the network guard
refuses every address that is not this Mac, by URL before sending and by
resolved address when connecting, and the update check is off.

| Host | When | What goes there |
| --- | --- | --- |
| `127.0.0.1` | A profile on a local server (Ollama, LM Studio, llama.cpp, vLLM). Detect probes ports 11434, 1234, 8080 and 8000 for `GET /v1/models` | The profile's requests. Nothing leaves the machine |
| `localhost` | A profile whose address names it. The guard lets through only the loopback addresses it resolves to | The same |
| `api.openai.com` | A profile made from the OpenAI preset, with AI on | That profile's requests and its key |
| `api.anthropic.com` | A profile made from the Anthropic preset | The same |
| `generativelanguage.googleapis.com` | A profile made from the Gemini preset | The same |
| `openrouter.ai` | A profile made from the OpenRouter preset | The same |
| `api.groq.com` | A profile made from the Groq preset | The same |
| `api.mistral.ai` | A profile made from the Mistral preset | The same |
| `api.deepseek.com` | A profile made from the DeepSeek preset | The same |
| `api.together.xyz` | A profile made from the Together preset | The same |
| Any address the user types | A profile the user made by hand | The same |
| `github.com` | Sparkle's update check: `releases/latest/download/appcast.xml` once a day, and the DMG of a new version when the user accepts it. Never in local-only mode (`UpdatePolicy`), and never from a build without a real update key, which is every build but a release. Planned: the signed data tables, from the same releases (not built) | The app's version and macOS version, in Sparkle's request. Nothing about the user's snippets |
| `release-assets.githubusercontent.com` | GitHub answers a release download with a redirect to its file storage, and Sparkle follows it | The same requests as to `github.com` |
| `objects.githubusercontent.com` | The same, for the older form of that redirect | The same |

A preset is only a starting point for a profile. No preset host is contacted
until the user saves a profile with it and switches AI on.

## Hosts named but never contacted

These are in the binary or the sources as text. Aralo sends no request to
them.

| Host | Why it is there |
| --- | --- |
| `platform.openai.com` | The page where OpenAI keys are made. The settings pane links to it, and a click opens it in the user's browser |
| `console.anthropic.com` | The same, for Anthropic |
| `aistudio.google.com` | The same, for Gemini |
| `console.groq.com` | The same, for Groq |
| `console.mistral.ai` | The same, for Mistral |
| `platform.deepseek.com` | The same, for DeepSeek |
| `api.together.ai` | The same, for Together |
| `api.example.com` | The placeholder text in the settings pane's address field |
| `docs.rs` | A link in an error message of `rustls`, the TLS library |

`github.com` is also named in `clap`'s error messages, the command line's
argument parser.

## At build time

Building Aralo contacts more hosts than running it does. None of these is
contacted by the app. Cargo fetches crates from crates.io.
`scripts/fetch-model.sh` fetches the embedding model from Hugging Face and
checks it against recorded checksums. CI pulls Ollama and llama.cpp container
images for the conformance job, and the nightly job calls the hosted endpoints
whose keys are repository secrets. The release workflow fetches Sparkle from
GitHub, and sends the app and the DMG to Apple's notary service.
