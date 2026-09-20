//! `on_key` cost with a 10,000-abbreviation library.
//!
//! Plan gate (M1): under 1 ms at p99 with 10,000 snippets. Criterion reports
//! the mean; `p99_gate` measures single calls and fails the run on a breach,
//! so `cargo bench -p aralo-engine` is usable as a CI check.
// The crate bans printing (P1); a benchmark report is the one exception, and it
// prints timings only.
#![allow(clippy::print_stdout, clippy::disallowed_macros)]

use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

use aralo_engine::{Abbreviation, Engine, KeyEvent, SnapshotBuilder, SnippetId, Trigger};
use criterion::{criterion_group, criterion_main, Criterion};

const SNIPPETS: u32 = 10_000;
const P99_BUDGET: Duration = Duration::from_millis(1);

/// A tiny deterministic generator, so runs are comparable across machines.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) as u32
    }
}

fn library() -> Engine {
    let mut random = Lcg(0x00A7_A100);
    let mut builder = SnapshotBuilder::new();
    for id in 0..SNIPPETS {
        // Shared prefixes (";", "x", "addr") keep the trie walk honest.
        let prefix = [";", "x", "addr", ""][(random.next() % 4) as usize];
        let length = 2 + random.next() % 8;
        let mut text = String::from(prefix);
        for _ in 0..length {
            text.push(char::from(b'a' + (random.next() % 26) as u8));
        }
        let mut abbreviation = Abbreviation::new(SnippetId(u128::from(id)), text);
        if id % 3 == 0 {
            abbreviation.trigger = Trigger::Immediate;
            abbreviation.whole_word = false;
        }
        builder.add(abbreviation);
    }
    let (snapshot, _) = builder.build();
    let mut engine = Engine::new();
    engine.set_snapshot(Arc::new(snapshot));
    engine.set_front_app("com.apple.TextEdit");
    engine
}

/// Prose-like typing: mostly letters, a delimiter every few characters.
fn keys(count: usize) -> Vec<KeyEvent> {
    let mut random = Lcg(0x5EED);
    (0..count)
        .map(|_| match random.next() % 12 {
            0 | 1 => KeyEvent::Char(' '),
            2 => KeyEvent::Char(';'),
            3 => KeyEvent::Backspace,
            _ => KeyEvent::Char(char::from(b'a' + (random.next() % 26) as u8)),
        })
        .collect()
}

fn p99_gate(engine: &mut Engine) {
    let keys = keys(200_000);
    let mut samples: Vec<Duration> = Vec::with_capacity(keys.len());
    for &key in &keys {
        let start = Instant::now();
        black_box(engine.on_key(black_box(key)));
        samples.push(start.elapsed());
    }
    samples.sort_unstable();
    let at = |q: f64| samples[((samples.len() - 1) as f64 * q) as usize];
    println!(
        "on_key, {SNIPPETS} abbreviations: p50 {:?}  p99 {:?}  p99.9 {:?}  max {:?}",
        at(0.50),
        at(0.99),
        at(0.999),
        at(1.0),
    );
    assert!(
        at(0.99) < P99_BUDGET,
        "on_key p99 {:?} is over the {:?} budget",
        at(0.99),
        P99_BUDGET
    );
}

fn bench(c: &mut Criterion) {
    let mut engine = library();
    p99_gate(&mut engine);

    let keys = keys(4096);
    let mut next = 0;
    c.bench_function("on_key/10k", |b| {
        b.iter(|| {
            let key = keys[next % keys.len()];
            next += 1;
            black_box(engine.on_key(black_box(key)))
        });
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
