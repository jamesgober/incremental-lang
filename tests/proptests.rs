//! Property tests: the core invariant of the crate.
//!
//! After any sequence of edits, a document's tree must equal the tree a fresh
//! parse of its text produces, and its text must equal the text the edits
//! describe. The documents are generated JSON — mostly well formed, so that
//! single-node reparses are attempted and taken — and the edits insert, delete,
//! and replace arbitrary fragments, including the ones that break lexing
//! (quotes, comment openers) and balance (stray brackets).

#[path = "../examples/common/json.rs"]
mod json;

use incremental_lang::{Builder, Document, Grammar, Span, Token};
use json::{Json, Kind};
use proptest::prelude::*;

/// JSONC with every node kind it can reparse on its own marked reparsable —
/// more candidates, so more chances for an unsafe splice to slip through.
#[derive(Clone, Copy)]
struct Eager;

impl Grammar for Eager {
    type Kind = Kind;
    const ROOT: Kind = Kind::Document;
    fn lex(&self, text: &str, tokens: &mut Vec<Token<Kind>>) {
        json::lex(text, tokens);
    }
    fn parse(&self, kind: Kind, b: &mut Builder<'_, Kind>) {
        json::parse(kind, b);
    }
    fn is_reparsable(&self, kind: Kind) -> bool {
        // `Error` is included on purpose: the grammar cannot rebuild one on its
        // own (it parses a document instead), so every attempt must be refused.
        matches!(
            kind,
            Kind::Object | Kind::Array | Kind::Member | Kind::Error
        )
    }
}

/// A JSON value, up to a few levels deep, with comments and odd spacing.
fn value() -> impl Strategy<Value = String> {
    let leaf = prop_oneof![
        Just("1".to_string()),
        Just("-2.5e3".to_string()),
        Just("true".to_string()),
        Just("null".to_string()),
        Just("\"é\"".to_string()),
        "\"[a-z ]{0,4}\"",
    ];
    leaf.prop_recursive(4, 48, 5, |inner| {
        let space = prop_oneof![
            Just(""),
            Just(" "),
            Just("\n  "),
            Just(" /* c */ "),
            Just(" // c\n")
        ];
        prop_oneof![
            (prop::collection::vec(inner.clone(), 0..5), space.clone())
                .prop_map(|(items, sep)| format!("[{sep}{}{sep}]", items.join(&format!(",{sep}")))),
            (prop::collection::vec(("[a-z]{1,3}", inner), 0..5), space).prop_map(
                |(members, sep)| {
                    let body: Vec<String> = members
                        .into_iter()
                        .map(|(k, v)| format!("\"{k}\":{sep}{v}"))
                        .collect();
                    format!("{{{sep}{}{sep}}}", body.join(&format!(",{sep}")))
                }
            ),
        ]
    })
}

/// A document: a top-level array of values, so most edits land inside some
/// reparsable node.
fn document() -> impl Strategy<Value = String> {
    prop::collection::vec(value(), 1..6)
        .prop_map(|items| format!("[\n  {}\n]\n", items.join(",\n  ")))
}

/// Text an edit inserts: mostly ordinary typing, plus the fragments that
/// disturb lexing or bracket balance.
fn fragment() -> impl Strategy<Value = String> {
    prop_oneof![
        6 => "[a-z0-9 ]{1,3}",
        3 => Just(String::new()),
        1 => Just("{".to_string()),
        1 => Just("}".to_string()),
        1 => Just("[".to_string()),
        1 => Just("]".to_string()),
        1 => Just(",".to_string()),
        1 => Just(":".to_string()),
        1 => Just("\"".to_string()),
        2 => Just("\"x\"".to_string()),
        1 => Just("/".to_string()),
        1 => Just("*".to_string()),
        1 => Just("//".to_string()),
        1 => Just("/*".to_string()),
        1 => Just("*/".to_string()),
        2 => Just("\n".to_string()),
        2 => Just("é".to_string()),
        2 => Just("{}".to_string()),
        2 => Just("[1, 2]".to_string()),
        2 => Just("\"k\": 1".to_string()),
    ]
}

/// An edit: where (an index into the text's character boundaries, wrapped),
/// how many characters to delete, and what to insert.
fn edit() -> impl Strategy<Value = (usize, usize, String)> {
    (any::<usize>(), 0usize..4, fragment())
}

/// Applies `(at, delete, insert)` to `model` and `doc`, then checks both
/// invariants.
fn apply_and_check<G: Grammar<Kind = Kind> + Copy>(
    grammar: G,
    doc: &mut Document<G>,
    model: &mut String,
    (at, delete, insert): &(usize, usize, String),
) -> Result<(), TestCaseError> {
    let boundaries: Vec<usize> = model
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(model.len()))
        .collect();
    let start_index = at % boundaries.len();
    let end_index = (start_index + delete).min(boundaries.len() - 1);
    let (start, end) = (boundaries[start_index], boundaries[end_index]);

    // Every other edit runs with a snapshot alive, so both the in-place and
    // the copy-on-write paths of the splice are exercised.
    let snapshot = (at % 2 == 0).then(|| (doc.tree().clone(), model.clone()));

    model.replace_range(start..end, insert);
    let outcome = doc.edit(Span::new(start as u32, end as u32), insert);
    prop_assert!(outcome.is_ok(), "edit refused: {outcome:?}");
    prop_assert_eq!(doc.text(), model.as_str());

    let fresh =
        Document::new(grammar, model.as_str()).map_err(|e| TestCaseError::fail(e.to_string()))?;
    prop_assert_eq!(doc.tree(), fresh.tree(), "diverged on {:?}", model);

    // A snapshot taken before the edit still describes the old text.
    if let Some((tree, text)) = snapshot {
        let old = Document::new(grammar, text.as_str())
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(&tree, old.tree(), "snapshot disturbed by edit");
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn test_incremental_matches_fresh_parse(
        text in document(),
        edits in prop::collection::vec(edit(), 1..24),
    ) {
        let mut doc = Document::new(Json, text.as_str()).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let mut model = text;
        for e in &edits {
            apply_and_check(Json, &mut doc, &mut model, e)?;
        }
        let stats = doc.stats();
        prop_assert_eq!(stats.edits, edits.len() as u64);
        prop_assert_eq!(stats.partial + stats.full, edits.len() as u64 + 1);
    }

    #[test]
    fn test_eager_grammar_matches_fresh_parse(
        text in document(),
        edits in prop::collection::vec(edit(), 1..24),
    ) {
        let mut doc = Document::new(Eager, text.as_str()).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let mut model = text;
        for e in &edits {
            apply_and_check(Eager, &mut doc, &mut model, e)?;
        }
    }

    #[test]
    fn test_garbage_text_matches_fresh_parse(
        text in prop::collection::vec(fragment(), 0..30).prop_map(|parts| parts.concat()),
        edits in prop::collection::vec(edit(), 1..16),
    ) {
        let mut doc = Document::new(Eager, text.as_str()).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let mut model = text;
        for e in &edits {
            apply_and_check(Eager, &mut doc, &mut model, e)?;
        }
    }

    #[test]
    fn test_tree_is_lossless(text in document()) {
        let doc = Document::new(Json, text.as_str()).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let mut at = 0u32;
        for token in doc.tree().root().tokens() {
            prop_assert_eq!(token.span.start().to_u32(), at);
            at = token.span.end().to_u32();
        }
        prop_assert_eq!(at as usize, text.len());
        // Every child lies inside its parent, in order, without gaps.
        for node in doc.tree().root().descendants() {
            let mut cursor = node.span().start().to_u32();
            for child in node.children() {
                prop_assert_eq!(child.span().start().to_u32(), cursor);
                cursor = child.span().end().to_u32();
            }
            prop_assert_eq!(cursor, node.span().end().to_u32());
        }
    }
}

/// The property tests above would pass vacuously if every edit fell back to a
/// full reparse. Replay a fixed editing session over generated-style JSON and
/// require that most edits were served by a single node.
#[test]
fn test_single_node_reparses_are_actually_taken() -> Result<(), Box<dyn std::error::Error>> {
    let mut text = String::from("{\n");
    for i in 0..40 {
        text.push_str(&format!("  \"k{i}\": [{i}, {{\"v\": \"s{i}\"}}, true],\n"));
    }
    text.push_str("  \"end\": null\n}\n");
    let mut doc = Document::new(Json, text)?;

    let mut edits = 0;
    for i in (0..40).step_by(3) {
        let needle = format!("\"s{i}\"");
        let at = doc.text().find(&needle).ok_or("needle")? as u32 + 2;
        doc.edit(Span::new(at, at), "z")?;
        let fresh = Document::new(Json, doc.text())?;
        assert_eq!(doc.tree(), fresh.tree());
        edits += 1;
    }
    let stats = doc.stats();
    assert_eq!(stats.partial, edits);
    assert_eq!(stats.full, 1);
    Ok(())
}
