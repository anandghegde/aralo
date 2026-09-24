//! aralo-embed against the reference implementation.
//!
//! `fixtures/embed/make.py` ran Hugging Face `tokenizers` and the `model2vec`
//! package over the same files these tests load, and wrote down what they
//! gave. A model distilled in Python means the same thing here only if every
//! id and every vector agrees, so these compare all of them.

use std::path::{Path, PathBuf};

use aralo_embed::{Model, Vectors};
use serde::Deserialize;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/embed")
}

fn tiny() -> Model {
    Model::open(&fixtures().join("tiny")).unwrap()
}

/// The same vocabulary with a clustered matrix, a mapping and token weights.
fn mapped() -> Model {
    let folder = fixtures().join("tiny");
    let read = |name: &str| std::fs::read(folder.join(name)).unwrap();
    Model::from_bytes(
        "mapped",
        &read("tokenizer.json"),
        &read("mapped.safetensors"),
        Some(&read("config.json")),
    )
    .unwrap()
}

#[derive(Deserialize)]
struct Tokens {
    text: String,
    ids: Vec<u32>,
}

#[derive(Deserialize)]
struct Embedding {
    text: String,
    vector: Vec<f32>,
}

fn read<T: for<'de> Deserialize<'de>>(name: &str) -> T {
    let text = std::fs::read_to_string(fixtures().join(name)).unwrap();
    serde_json::from_str(&text).unwrap()
}

#[test]
fn every_text_tokenizes_to_the_ids_the_reference_gives() {
    let model = tiny();
    let cases: Vec<Tokens> = read("tokens.json");
    assert!(cases.len() > 30);
    let mut wrong = Vec::new();
    for case in &cases {
        let ids = model.tokenize(&case.text);
        if ids != case.ids {
            wrong.push(format!(
                "{:?}\n  reference {:?}\n  here      {:?}",
                case.text.chars().take(80).collect::<String>(),
                case.ids,
                ids
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

fn same_vectors(model: &Model, cases: &[Embedding]) {
    let mut wrong = Vec::new();
    for case in cases {
        let vector = model.embed(&case.text);
        let off = vector
            .iter()
            .zip(&case.vector)
            .map(|(here, there)| (here - there).abs())
            .fold(0.0f32, f32::max);
        if vector.len() != case.vector.len() || off > 1e-4 {
            wrong.push(format!(
                "{:?}\n  reference {:?}\n  here      {:?}",
                case.text.chars().take(80).collect::<String>(),
                case.vector,
                vector
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[derive(Deserialize)]
struct Embeddings {
    #[serde(rename = "model.safetensors")]
    plain: Vec<Embedding>,
    #[serde(rename = "mapped.safetensors")]
    mapped: Vec<Embedding>,
}

#[test]
fn every_text_embeds_to_the_vector_the_reference_gives() {
    let expected: Embeddings = read("embeddings.json");
    same_vectors(&tiny(), &expected.plain);
    same_vectors(&mapped(), &expected.mapped);
}

#[test]
fn a_model_is_known_by_its_name_and_its_files() {
    let model = tiny();
    assert!(model.id().starts_with("tiny@"), "{}", model.id());
    assert_eq!(model.dimensions(), 8);
    assert_ne!(
        model.id().split('@').nth(1),
        mapped().id().split('@').nth(1)
    );
    // Loading twice is the same model.
    assert_eq!(model.id(), tiny().id());
}

#[test]
fn a_text_the_model_knows_nothing_of_is_similar_to_nothing() {
    let model = tiny();
    let nothing = model.embed("ภาษาไทย");
    assert!(nothing.iter().all(|&value| value == 0.0));
    let mut vectors = Vectors::new(model.dimensions());
    assert!(!vectors.push("thai", &nothing));
    assert!(vectors.push("hello", &model.embed("hello world")));
    assert!(vectors.nearest(&nothing, 5, -1.0).is_empty());
    let found = vectors.nearest(&model.embed("hello world"), 5, 0.99);
    assert_eq!(found.len(), 1);
}

#[test]
fn a_folder_that_is_not_a_model_says_what_is_missing() {
    let error = Model::open(&fixtures()).unwrap_err();
    assert!(error.to_string().contains("tokenizer.json"), "{error}");
}
