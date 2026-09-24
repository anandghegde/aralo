"""Builds the fixtures that hold aralo-embed to the reference implementation.

    python3 -m venv venv && venv/bin/pip install tokenizers model2vec safetensors numpy
    venv/bin/python fixtures/embed/make.py

README.md in this folder says what each file is and where the tokenizer came
from. The numbers are random and mean nothing; what the fixtures pin is that
the Rust tokenizer and encoder turn the same files into the same numbers as
Python.
"""

import json
import pathlib
import shutil
import tempfile

import numpy as np
from model2vec import StaticModel
from safetensors.numpy import save_file
from tokenizers import Tokenizer

HERE = pathlib.Path(__file__).parent
TINY = HERE / "tiny"

TEXTS = [
    "hello world",
    "Hello, World!",
    "Thanks for your order. We have issued a full refund to your card.",
    "Can I get my money back?",
    "unaffable unbelievably antidisestablishmentarianism",
    "Café, naïve, résumé, Grüße, Ærøskøbing, İstanbul, straße",
    "é and é are the same letter",
    "ΟΔΟΣ Αθήνα όνομα",
    "Москва и Санкт-Петербург",
    "我爱北京天安门 and 東京",
    "日本語のテキストとカタカナ",
    "한국어 텍스트",
    "नमस्ते दुनिया",
    "مرحبا بالعالم",
    "ภาษาไทย",
    "emoji 😀 in the middle😀of words",
    "zero​width and soft­hyphen and ‏marks",
    "tabs\tnewlines\nand\r\ncarriage returns　and spaces",
    "control \x00 \x07 \x1b characters and � replacement",
    "{{field: order number}} and {{clipboard}} and {{date: %Y-%m-%d}}",
    "[CLS] special [SEP] tokens [MASK] in text [PAD] [UNK] [cls]",
    "a[SEP]b[SEP][SEP]c",
    "email me at someone@example.com or visit https://example.com/a?b=c&d=e",
    "prices: $5.99, €10, £3 — 50% off!!! (really) [maybe] {no} <yes>",
    "don't won't y'all rock'n'roll",
    "1234567890 3.14159 1,000,000 2026-09-24",
    "ﬁne ﬂow ＡＢＣ ｱｲｳ ①②③ ½",
    "x" * 99 + " " + "y" * 100 + " " + "z" * 101,
    "𠀋 𪚲 𫝀 𫠝 𬺰 𰀀",
    "",
    "   ",
    "!!!",
    "the quick brown fox jumps over the lazy dog " * 300,
]


def random_models():
    tokenizer = json.loads((TINY / "tokenizer.json").read_text())
    rows = max(tokenizer["model"]["vocab"].values()) + 1
    rng = np.random.default_rng(20260924)
    save_file(
        {"embeddings": rng.integers(-127, 128, size=(rows, 8), dtype=np.int8)},
        str(TINY / "model.safetensors"),
    )
    save_file(
        {
            "embeddings": rng.standard_normal((32, 8)).astype(np.float16),
            "mapping": rng.integers(0, 32, size=rows, dtype=np.int32),
            "weights": rng.uniform(0.5, 1.5, size=rows).astype(np.float32),
        },
        str(TINY / "mapped.safetensors"),
    )
    (TINY / "config.json").write_text(json.dumps({"normalize": True}, indent=2) + "\n")


def reference(tensors):
    """The model2vec package's own reading of the fixture."""
    with tempfile.TemporaryDirectory() as folder:
        folder = pathlib.Path(folder)
        shutil.copy(TINY / "tokenizer.json", folder / "tokenizer.json")
        shutil.copy(TINY / "config.json", folder / "config.json")
        shutil.copy(TINY / tensors, folder / "model.safetensors")
        return StaticModel.from_pretrained(str(folder))


def main():
    random_models()
    tokenizer = Tokenizer.from_file(str(TINY / "tokenizer.json"))
    tokens = [
        {"text": text, "ids": tokenizer.encode(text, add_special_tokens=False).ids}
        for text in TEXTS
    ]
    (HERE / "tokens.json").write_text(json.dumps(tokens, ensure_ascii=False, indent=1) + "\n")

    embeddings = {}
    for tensors in ("model.safetensors", "mapped.safetensors"):
        model = reference(tensors)
        vectors = model.encode(TEXTS)
        embeddings[tensors] = [
            {"text": text, "vector": [round(float(value), 7) for value in vector]}
            for text, vector in zip(TEXTS, vectors)
        ]
    (HERE / "embeddings.json").write_text(
        json.dumps(embeddings, ensure_ascii=False, indent=1) + "\n"
    )


if __name__ == "__main__":
    main()
