# Embedding fixtures

What holds `aralo-embed` to the reference implementation of static embeddings
(ADR-0016). `crates/aralo-embed/tests/reference.rs` loads the files here and
checks every token id and every vector against what the Python packages gave.

| File | What it is |
| --- | --- |
| `tiny/tokenizer.json` | The WordPiece tokenizer of BAAI's bge-base-en-v1.5 with its unused tokens removed: the vocabulary MinishLab's potion models use. From the model2vec-rs test fixtures (MIT, The Minish Lab) |
| `tiny/config.json` | `normalize: true`, as the real model has |
| `tiny/model.safetensors` | An `I8` embedding matrix, 8 random columns per token |
| `tiny/mapped.safetensors` | An `F16` matrix of 32 rows, a vocabulary mapping and token weights, as a vocabulary-quantized model has |
| `tokens.json` | The ids Hugging Face `tokenizers` gives 33 texts chosen to break a tokenizer |
| `embeddings.json` | The vectors the `model2vec` package gives the same texts, for both models |
| `make.py` | Writes all of the above but the tokenizer |

The numbers in the models are random and mean nothing. What they pin is that
the Rust code turns the same files into the same numbers as Python. The core's
tests use `tiny/` too: a static model averages its tokens, so "world hello" is
the vector "hello world" is, while no word search finds one in the other.

To regenerate, after changing a text or adopting a model with another
vocabulary:

```sh
python3 -m venv venv && venv/bin/pip install tokenizers model2vec safetensors numpy
venv/bin/python fixtures/embed/make.py
```
