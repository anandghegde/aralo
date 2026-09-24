# ADR-0016: Search by meaning runs a static embedding model, in pure Rust

- Status: Accepted (not yet confirmed by the project owner)
- Date: 2026-09-24
- Answers spike S5. Supersedes the embeddings row of
  [ADR-0008](0008-library-choices.md).

## Context

PRD A4 asks for search by meaning: typing "money back" finds the refund reply
that never uses either word. The plan's done-when for task 4.8 adds the
constraint that matters most here: *with no network call*. The model has to
ship with the app and run on the user's machine.

ADR-0008 picked `candle` running a quantised MiniLM-class transformer, with
`ort` as the fallback, provisionally, until spike S5 measured it. A
transformer reads a text and works out what each word means in it; that is
the best quality available, and it costs a matrix engine, a model of 23 to
90 MB, and milliseconds per snippet.

A snippet library is not a document corpus. It is a few hundred to a few
thousand short texts, searched as the user types, by an app whose other job
is to sit on the keyboard path and cost nothing. The question S5 asked, *is
it fast and small enough*, has a second half: is a transformer's quality worth
what it costs for texts this short?

Static embeddings answer differently. MinishLab's Model2Vec distils a sentence
transformer into one vector per token; embedding a text is a tokenizer pass
and an average. Their `potion-base-8M` is distilled from BAAI's
bge-base-en-v1.5, is MIT licensed, and is 30 MB as `f32`. By MinishLab's own
benchmarks it scores below MiniLM-class transformers on MTEB, and it is
hundreds of times faster.

## Method

The embedding runtime was written first (`crates/aralo-embed`), then held to
the reference implementation, then timed.

- **Conformance.** `fixtures/embed/make.py` runs Hugging Face `tokenizers`
  and the `model2vec` Python package over a tiny model with the real bge
  vocabulary: 33 texts chosen to break a tokenizer (accents, combining marks,
  CJK, Hangul, Devanagari, Arabic, Thai, emoji, control and zero-width
  characters, special tokens inside text, words over 100 characters,
  placeholders, a text over 512 tokens), for an `I8` matrix and for a
  vocabulary-quantized `F16` one with a mapping and token weights. Every id
  and every vector agrees (`crates/aralo-embed/tests/reference.rs`). A
  further 4,000 random strings drawn from twelve scripts tokenized to the
  same ids as the reference; that run is not kept as a test.
- **Cost.** A model the size of `potion-base-8M` (29,528 × 256 `f32`, the
  real tokenizer, random weights, since speed does not depend on the values),
  release build, one core of the Linux container CI's jobs resemble.

## Measurements

| What | Measured |
| --- | --- |
| Load the model (read, parse tokenizer, decode weights) | 61 ms |
| Resident memory once loaded | 38 MB (69 MB at the peak of loading) |
| Embed one snippet of 190 characters | 24 µs |
| Embed 5,000 such snippets | 122 ms |
| Embed a two-word query | 1.7 µs |
| Scan 10,000 vectors for the nearest 50 | 1.8 ms |

How the real model ranks the twelve snippets of `fixtures/search/library`
(cosine similarity; CI prints the full table on every run):

| Query | The snippet it should find | The next one |
| --- | --- | --- |
| give the customer their money back | Refund issued, 0.389 | Email signature, 0.331 |
| my package is late | Shipping delay, 0.435 | Birthday wishes, 0.222 |
| forgot my login | Password reset, 0.491 | Subscription cancelled, 0.232 |
| notes from our call | Meeting follow-up, 0.482 | Refund issued, 0.281 |

The right snippet comes first every time, but the scores are low and close:
a static model averages every token of a snippet, so a short query never
scores near 1, and unrelated snippets reach 0.3. Hence a bar of 0.25, hits by
meaning after every literal hit, and at most five of them.

`candle` with MiniLM was not measured. The weights could not be fetched into
the environment the spike ran in, and the numbers above leave no problem for
a transformer to solve: a library of 5,000 snippets embeds in the time a
transformer spends on a few dozen.

## Decision

- **A static model, `potion-base-8M`, run by Aralo's own code.**
  `aralo-embed` reads the model's `tokenizer.json`, `model.safetensors` and
  `config.json`, tokenizes with BERT's WordPiece exactly as the reference
  does, and averages token vectors as Model2Vec does. No ML framework, no
  `tokenizers` crate, no dynamic library to sign.
- **The model ships inside the app and never downloads at runtime.**
  `scripts/fetch-model.sh` fetches it at build time from Hugging Face and
  checks each file against a recorded SHA-256; `make app` bundles it. The
  weights are not in Git.
- **Nothing in `aralo-embed` can reach the network.** Its whole dependency
  closure is an allow-list in `scripts/check-deps.sh`, beside the engine's
  and the template's, and no crate on it opens a socket.
- **Vectors are kept, keyed by content hash and model.** The `vectors` table
  was reserved for this in M2. A model is known by its name and a hash of its
  files, so a new model never reads an old one's vectors, and the old ones
  are pruned.
- **Hits by meaning rank after every literal hit.** Search keeps the order it
  can explain in one sentence ([library search](../architecture.md)), and
  meaning comes last, most similar first, at most five, above a similarity
  bar of 0.25.

## Consequences

- Embedding is cheap enough to do on the indexer thread after every sync, and
  a query is embedded on every keystroke.
- The model has no context. "Bank" is one vector whatever the river says, and
  word order does not count: "dog bites man" means what "man bites dog"
  means. For short snippets found by a few words, that is rarely the
  difference.
- The vocabulary is English and lower-cased. Other languages tokenize into
  fragments and find little by meaning; their literal search is unchanged.
  MinishLab's multilingual model is sixteen times the size, and is a later
  decision.
- The tokenizer is Aralo's to keep correct. The reference fixtures pin it,
  and `make.py` regenerates them if a model with another vocabulary is
  adopted. Only BERT's WordPiece is read; any other tokenizer is refused, not
  approximated.
- The checksums were recorded from the first download, on CI, of
  `minishlab/potion-base-8M` at commit `bf8b056`. A download that does not
  match them fails.

## Alternatives rejected

- **`candle` with a MiniLM-class transformer** (ADR-0008's choice). A matrix
  engine and its dependencies in the core, 23 to 90 MB of weights, and
  milliseconds per snippet, for quality short texts barely use.
- **`ort` with ONNX Runtime.** The same model cost, plus a C++ dynamic
  library to sign, notarise and keep patched.
- **The `tokenizers` crate.** A regex engine, a thread pool and four other
  tokenizer models to run one WordPiece tokenizer; about 200 lines do the
  same, held to the reference by test.
- **WordLlama's weights**, which the `wordllama` Python package ships. They
  are derived from Llama 2's token embeddings, which would bring the Llama 2
  Community License into an Apache-2.0 app.
- **Weights in the repository.** 30 MB in every clone's history, and again
  for every model update.
- **Downloading the model on first use.** The app would make a network call
  for search, which is the one thing the PRD says it must not.
