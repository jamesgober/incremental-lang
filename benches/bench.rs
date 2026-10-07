//! Criterion benchmarks: whole-document parsing against incremental edits.
//!
//! Every edit benchmark types one character inside a small object in the
//! middle of the document and erases it again, so the document is the same at
//! the start of every iteration; the reported time covers both edits.
//!
//! - `parse/<size>` — `Document::new`: lex and parse everything.
//! - `edit/<size>` — the keystroke pair through incremental reparsing.
//! - `edit_full/<size>` — the same pair with no reparsable kinds, so every
//!   edit is a whole-document reparse: the baseline incremental reparsing
//!   replaces.
//! - `edit_flat/<size>` — the keystroke pair inside an element of one very
//!   wide array, which measures the cost of copying a wide child array on the
//!   path to the edit.
//! - `edit_deep/<terms>` — a pair of edits inside a small list at the bottom
//!   of a left-associative chain of `<terms>` non-reparsable sums, which
//!   measures the walk down to a deep edit and the splice back up.
//! - `walk/tokens` and `snapshot/clone` — reading the tree and keeping a
//!   version of it.

#[path = "../examples/common/json.rs"]
mod json;

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
use json::{Json, Kind};

/// JSONC with nothing reparsable: every edit reparses the whole document.
#[derive(Clone, Copy)]
struct FullOnly;

impl Grammar for FullOnly {
    type Kind = Kind;
    const ROOT: Kind = Kind::Document;
    fn lex(&self, text: &str, tokens: &mut Vec<Token<Kind>>) {
        json::lex(text, tokens);
    }
    fn parse(&self, kind: Kind, b: &mut Builder<'_, Kind>) {
        json::parse(kind, b);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ck {
    Root,
    List,
    Sum,
    Open,
    Close,
    Plus,
    Word,
}

impl TokenKind for Ck {}

/// `[a+b+c]`: lists of left-associative sums, built with checkpoints, so a
/// chain of `n` terms nests `n` sum nodes.
struct Chain;

impl Grammar for Chain {
    type Kind = Ck;
    const ROOT: Ck = Ck::Root;
    fn lex(&self, text: &str, tokens: &mut Vec<Token<Ck>>) {
        for (i, byte) in text.bytes().enumerate() {
            let kind = match byte {
                b'[' => Ck::Open,
                b']' => Ck::Close,
                b'+' => Ck::Plus,
                _ => Ck::Word,
            };
            tokens.push(Token::new(kind, Span::new(i as u32, i as u32 + 1)));
        }
    }
    fn parse(&self, kind: Ck, b: &mut Builder<'_, Ck>) {
        if kind == Ck::List {
            return chain_list(b);
        }
        b.start(Ck::Root);
        while !b.at_end() {
            if b.at(Ck::Open) {
                chain_list(b);
            } else {
                b.bump();
            }
        }
        b.finish();
    }
    fn is_reparsable(&self, kind: Ck) -> bool {
        kind == Ck::List
    }
}

fn chain_term(b: &mut Builder<'_, Ck>) {
    if b.at(Ck::Open) {
        chain_list(b);
    } else {
        b.bump();
    }
}

fn chain_list(b: &mut Builder<'_, Ck>) {
    b.start(Ck::List);
    b.bump();
    if !b.at(Ck::Close) && !b.at_end() {
        let lhs = b.checkpoint();
        chain_term(b);
        while b.eat(Ck::Plus) {
            chain_term(b);
            b.start_at(lhs, Ck::Sum);
            b.finish();
        }
    }
    b.eat(Ck::Close);
    b.finish();
}

/// One record of the generated documents (about 130 bytes).
fn record(i: usize) -> String {
    format!(
        "{{\"id\": {i}, \"name\": \"item {i}\", \"tags\": [\"a\", \"b\"], \
         \"nested\": {{\"x\": 1.5, \"y\": [1, 2, 3]}}}}"
    )
}

/// About `bytes` of JSON, grouped: a top-level object of sections, each an
/// array of 32 records — the nesting real configuration and data files have.
fn grouped(bytes: usize) -> String {
    let mut text = String::from("{\n");
    let mut i = 0;
    let mut section = 0;
    while text.len() < bytes {
        text.push_str(&format!("  \"section{section}\": [\n"));
        for _ in 0..32 {
            text.push_str(&format!("    {},\n", record(i)));
            i += 1;
        }
        text.push_str("    null\n  ],\n");
        section += 1;
    }
    text.push_str("  \"end\": true\n}\n");
    text
}

/// About `bytes` of JSON as one flat array of records.
fn flat(bytes: usize) -> String {
    let mut text = String::from("[\n");
    let mut i = 0;
    while text.len() < bytes {
        text.push_str(&format!("  {},\n", record(i)));
        i += 1;
    }
    text.push_str("  null\n]\n");
    text
}

/// The offset inside the `"x": 1.5` of the record nearest the middle.
fn middle_offset(text: &str) -> u32 {
    let from = text.len() / 2;
    let at = text[from..].find("\"x\": 1").map_or(0, |i| from + i + 6);
    at as u32
}

/// Types `9` at `at` and erases it.
fn keystroke<G: Grammar<Kind = Kind>>(doc: &mut Document<G>, at: u32) {
    let _typed = black_box(doc.edit(Span::new(at, at), "9"));
    let _erased = black_box(doc.edit(Span::new(at, at + 1), ""));
}

const SIZES: [(usize, &str); 3] = [(10_000, "10KB"), (100_000, "100KB"), (1_000_000, "1MB")];

fn bench_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("parse");
    for (bytes, label) in SIZES {
        let text = grouped(bytes);
        group.throughput(Throughput::Bytes(text.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(label), &text, |b, text| {
            b.iter(|| Document::new(Json, black_box(text.as_str())));
        });
    }
    group.finish();
}

fn bench_edit(c: &mut Criterion) {
    let mut group = c.benchmark_group("edit");
    for (bytes, label) in SIZES {
        let text = grouped(bytes);
        let at = middle_offset(&text);
        let Ok(mut doc) = Document::new(Json, text) else {
            continue;
        };
        group.bench_function(BenchmarkId::from_parameter(label), |b| {
            b.iter(|| keystroke(&mut doc, at));
        });
    }
    group.finish();
}

fn bench_edit_full(c: &mut Criterion) {
    let mut group = c.benchmark_group("edit_full");
    group.sample_size(20);
    for (bytes, label) in SIZES {
        let text = grouped(bytes);
        let at = middle_offset(&text);
        let Ok(mut doc) = Document::new(FullOnly, text) else {
            continue;
        };
        group.bench_function(BenchmarkId::from_parameter(label), |b| {
            b.iter(|| keystroke(&mut doc, at));
        });
    }
    group.finish();
}

fn bench_edit_flat(c: &mut Criterion) {
    let mut group = c.benchmark_group("edit_flat");
    for (bytes, label) in SIZES {
        let text = flat(bytes);
        let at = middle_offset(&text);
        let Ok(mut doc) = Document::new(Json, text) else {
            continue;
        };
        group.bench_function(BenchmarkId::from_parameter(label), |b| {
            b.iter(|| keystroke(&mut doc, at));
        });
    }
    group.finish();
}

fn bench_edit_deep(c: &mut Criterion) {
    let mut group = c.benchmark_group("edit_deep");
    for terms in [1_000usize, 10_000, 40_000] {
        // `[[x]+x+x+...]`: the small list at the bottom is the only node
        // reparsed; the rest is the walk down and the splice back up.
        let mut text = String::from("[[x]");
        for _ in 0..terms {
            text.push_str("+x");
        }
        text.push(']');
        let Ok(mut doc) = Document::new(Chain, text) else {
            continue;
        };
        group.bench_function(BenchmarkId::from_parameter(terms), |b| {
            b.iter(|| {
                // Retype the bottom term and change it back: two edits.
                let _typed = black_box(doc.edit(Span::new(2, 3), "y"));
                let _restored = black_box(doc.edit(Span::new(2, 3), "x"));
            });
        });
    }
    group.finish();
}

fn bench_read(c: &mut Criterion) {
    let text = grouped(100_000);
    let Ok(doc) = Document::new(Json, text) else {
        return;
    };
    let mut group = c.benchmark_group("walk");
    group.throughput(Throughput::Bytes(doc.text().len() as u64));
    group.bench_function("tokens/100KB", |b| {
        b.iter(|| black_box(doc.tree().root().tokens().count()));
    });
    group.bench_function("descendants/100KB", |b| {
        b.iter(|| black_box(doc.tree().root().descendants().count()));
    });
    group.finish();

    c.bench_function("snapshot/clone", |b| {
        b.iter(|| black_box(doc.tree().clone()));
    });
}

criterion_group!(
    benches,
    bench_parse,
    bench_edit,
    bench_edit_full,
    bench_edit_flat,
    bench_edit_deep,
    bench_read
);
criterion_main!(benches);
