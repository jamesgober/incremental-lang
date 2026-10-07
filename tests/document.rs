//! Integration tests: documents built on the JSONC grammar, edited through the
//! public API, and checked against fresh parses.

#[path = "../examples/common/json.rs"]
mod json;

use incremental_lang::{Builder, Document, Element, Error, Grammar, Span, Token, TokenKind};
use json::{Json, Kind};

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Asserts the document's tree is the tree a fresh parse of its text yields,
/// and that the tree is lossless.
fn assert_consistent<G: Grammar<Kind = Kind> + Copy>(doc: &Document<G>, grammar: G) -> TestResult {
    let fresh = Document::new(grammar, doc.text())?;
    assert_eq!(
        doc.tree(),
        fresh.tree(),
        "incremental tree diverged for {:?}",
        doc.text()
    );
    let rebuilt: String = doc
        .tree()
        .root()
        .tokens()
        .map(|t| &doc.text()[t.span.start().to_usize()..t.span.end().to_usize()])
        .collect();
    assert_eq!(rebuilt, doc.text());
    Ok(())
}

fn kinds_of(doc: &Document<Json>, kind: Kind) -> Vec<Span> {
    doc.tree()
        .root()
        .descendants()
        .filter(|n| n.kind() == kind)
        .map(|n| n.span())
        .collect()
}

#[test]
fn test_new_builds_expected_structure() -> TestResult {
    let doc = Document::new(Json, r#"{"a": [1, 2], "b": null}"#)?;
    let root = doc.tree().root();
    assert_eq!(root.kind(), Kind::Document);
    assert_eq!(kinds_of(&doc, Kind::Object), [Span::new(0, 24)]);
    assert_eq!(kinds_of(&doc, Kind::Array), [Span::new(6, 12)]);
    assert_eq!(kinds_of(&doc, Kind::Member).len(), 2);
    assert!(kinds_of(&doc, Kind::Error).is_empty());
    Ok(())
}

#[test]
fn test_edit_inside_array_reparses_only_the_array() -> TestResult {
    let mut doc = Document::new(Json, r#"{"a": [1, 2], "b": {"c": 3}}"#)?;
    let edit = doc.edit(Span::new(10, 11), "20")?;
    assert!(!edit.is_full());
    assert_eq!(edit.kind(), Kind::Array);
    assert_eq!(edit.span(), Span::new(6, 13));
    assert_eq!(doc.text(), r#"{"a": [1, 20], "b": {"c": 3}}"#);
    assert_eq!(doc.stats().partial, 1);
    assert_consistent(&doc, Json)
}

#[test]
fn test_edit_picks_innermost_reparsable_node() -> TestResult {
    let mut doc = Document::new(Json, r#"[[[1]], [2]]"#)?;
    let edit = doc.edit(Span::new(3, 4), "7")?;
    assert_eq!(edit.span(), Span::new(2, 5)); // the innermost `[1]`
    assert_consistent(&doc, Json)
}

#[test]
fn test_untouched_subtrees_are_shared_with_snapshot() -> TestResult {
    let mut doc = Document::new(Json, r#"[{"x": 1}, {"y": 2}]"#)?;
    let before = doc.tree().clone();
    doc.edit(Span::new(17, 18), "3")?;
    // The first object is identical in both trees: same kinds, same spans.
    let first = |tree: &incremental_lang::Tree<Kind>| {
        tree.root()
            .descendants()
            .find(|n| n.kind() == Kind::Object)
            .map(|n| n.span())
    };
    assert_eq!(first(&before), first(doc.tree()));
    assert_consistent(&doc, Json)
}

#[test]
fn test_unbalancing_edit_widens_to_enclosing_node() -> TestResult {
    let mut doc = Document::new(Json, r#"[{"a": [1, 2], "b": 3}, "padding padding"]"#)?;
    // A second `[` opens a nested array whose `]` closes it, leaving the outer
    // array unclosed: reparsed alone, it runs off its end and is refused. The
    // enclosing object absorbs the change.
    let edit = doc.edit(Span::new(8, 8), "[")?;
    assert!(!edit.is_full());
    assert_eq!(edit.kind(), Kind::Object);
    assert_eq!(edit.span(), Span::new(1, 23));
    assert_eq!(doc.stats().rejected, 1);
    assert_consistent(&doc, Json)
}

#[test]
fn test_unterminated_string_on_one_line_falls_back_to_full_reparse() -> TestResult {
    let mut doc = Document::new(Json, r#"[["a"], ["b"]]"#)?;
    // A stray quote re-pairs every string after it on the line.
    let edit = doc.edit(Span::new(3, 3), "\"")?;
    assert!(edit.is_full());
    assert_eq!(edit.kind(), Kind::Document);
    assert_eq!(edit.span(), Span::new(0, 15));
    assert_consistent(&doc, Json)
}

#[test]
fn test_junk_inside_a_list_stays_local() -> TestResult {
    let mut doc = Document::new(Json, "[1, 2, 3]\n{\"k\": [4]}")?;
    let edit = doc.edit(Span::new(5, 5), "oops")?;
    assert!(!edit.is_full());
    assert_eq!(edit.kind(), Kind::Array);
    assert_eq!(kinds_of(&doc, Kind::Error).len(), 1);
    assert_consistent(&doc, Json)
}

#[test]
fn test_rejected_attempts_are_counted() -> TestResult {
    let mut doc = Document::new(Json, r#"["padding padding padding", {"a": [1, 2]}]"#)?;
    // A `}` inside the inner array ends the array early, then the object
    // early: both attempts are refused, and the outer array's attempt would
    // exceed the lexing budget, so the document reparses everything.
    let edit = doc.edit(Span::new(37, 37), "}")?;
    assert!(edit.is_full());
    assert_eq!(doc.stats().rejected, 2);
    assert_consistent(&doc, Json)
}

#[test]
fn test_failed_attempts_stop_at_the_lexing_budget() -> TestResult {
    let mut doc = Document::new(Json, r#"{"a": [1, 2]}"#)?;
    // The array's attempt is refused. The object's window would take the
    // bytes lexed on attempts past the document's length, so it is skipped.
    let edit = doc.edit(Span::new(9, 9), "}")?;
    assert!(edit.is_full());
    let stats = doc.stats();
    assert_eq!(stats.rejected, 1);
    // Initial parse + one array window + one full reparse.
    assert!(stats.bytes <= 13 + 2 * 14);
    assert_consistent(&doc, Json)
}

#[test]
fn test_unterminated_string_inside_node_is_caught() -> TestResult {
    let mut doc = Document::new(Json, "[\"a\", \"b\"]\n[1]")?;
    // Delete the closing quote of "b": the string now runs to the line end and
    // swallows the `]`.
    doc.edit(Span::new(8, 9), "")?;
    assert_consistent(&doc, Json)?;
    assert!(!kinds_of(&doc, Kind::Error).is_empty());
    Ok(())
}

#[test]
fn test_block_comment_opened_inside_node_is_caught() -> TestResult {
    let mut doc = Document::new(Json, "[1, 2]\n[3]")?;
    // `/*` with no `*/` turns the rest of the file into a comment.
    let edit = doc.edit(Span::new(3, 3), "/*")?;
    assert!(edit.is_full());
    assert_consistent(&doc, Json)
}

#[test]
fn test_neighbour_token_growth_is_caught() -> TestResult {
    // A grammar that reparses members, whose last token is a bare value.
    let mut doc = Document::new(Eager, r#"{"a": 1}"#)?;
    for (span, text) in [
        (Span::new(5, 5), "tr"),
        (Span::new(1, 1), " "),
        (Span::new(6, 6), "ue"),
        (Span::new(2, 3), ""),
    ] {
        doc.edit(span, text)?;
        assert_consistent(&doc, Eager)?;
    }
    Ok(())
}

#[test]
fn test_nodes_at_document_edges_reparse() -> TestResult {
    // The array is the whole document: no token before or after it.
    let mut doc = Document::new(Json, "[1, 2, 3]")?;
    let edit = doc.edit(Span::new(4, 5), "9")?;
    assert!(!edit.is_full());
    assert_eq!(edit.span(), Span::new(0, 9));
    assert_consistent(&doc, Json)
}

#[test]
fn test_edits_with_multibyte_text() -> TestResult {
    let mut doc = Document::new(Json, r#"["é", "ü"]"#)?;
    doc.edit(Span::new(2, 4), "ñö")?;
    assert_eq!(doc.text(), r#"["ñö", "ü"]"#);
    assert_consistent(&doc, Json)?;
    assert_eq!(
        doc.edit(Span::new(3, 3), "x"),
        Err(Error::NotCharBoundary { offset: 3 })
    );
    Ok(())
}

#[test]
fn test_out_of_bounds_edit_is_refused_without_change() -> TestResult {
    let mut doc = Document::new(Json, "[1]")?;
    let before = doc.tree().clone();
    let stats = doc.stats();
    assert_eq!(
        doc.edit(Span::new(1, 9), ""),
        Err(Error::OutOfBounds {
            span: Span::new(1, 9),
            len: 3
        })
    );
    assert_eq!(doc.text(), "[1]");
    assert_eq!(doc.tree(), &before);
    assert_eq!(doc.stats(), stats);
    Ok(())
}

#[test]
fn test_whole_text_replacement() -> TestResult {
    let mut doc = Document::new(Json, "[1]")?;
    let len = doc.text().len() as u32;
    let edit = doc.edit(Span::new(0, len), r#"{"k": true}"#)?;
    assert!(edit.is_full());
    assert_consistent(&doc, Json)
}

#[test]
fn test_empty_document_and_insertions_into_it() -> TestResult {
    let mut doc = Document::new(Json, "")?;
    assert_eq!(doc.tree().root().span(), Span::new(0, 0));
    assert_eq!(doc.tree().root().children().len(), 0);
    doc.edit(Span::new(0, 0), "[]")?;
    doc.edit(Span::new(1, 1), "1")?;
    assert_eq!(doc.text(), "[1]");
    assert_consistent(&doc, Json)
}

#[test]
fn test_typing_a_document_character_by_character() -> TestResult {
    let target = "{\n  \"name\": \"incremental\",\n  \"tags\": [\"a\", \"b\"],\n  \"n\": 3\n}\n";
    let mut doc = Document::new(Json, "")?;
    for (offset, c) in target.char_indices() {
        let at = offset as u32;
        doc.edit(Span::new(at, at), c.encode_utf8(&mut [0; 4]))?;
        assert_consistent(&doc, Json)?;
    }
    assert_eq!(doc.text(), target);
    Ok(())
}

#[test]
fn test_typing_inside_a_large_document_is_mostly_partial() -> TestResult {
    let mut text = String::from("[\n");
    for i in 0..200 {
        text.push_str(&format!("  {{\"id\": {i}, \"tags\": [\"x\", \"y\"]}},\n"));
    }
    text.push_str("  null\n]\n");
    let mut doc = Document::new(Json, text)?;

    // Edit the inside of a member value near the middle, 50 times.
    let anchor = doc.text().find("\"id\": 100").ok_or("anchor")? as u32 + 7;
    for _ in 0..50 {
        doc.edit(Span::new(anchor, anchor), "9")?;
    }
    assert_consistent(&doc, Json)?;
    let stats = doc.stats();
    assert_eq!(stats.edits, 50);
    assert_eq!(stats.partial, 50);
    // Each edit relexed one small object plus its neighbours, not the file.
    assert!(stats.bytes < doc.text().len() as u64 + 50 * 128);
    Ok(())
}

#[test]
fn test_deeply_nested_document() -> TestResult {
    let depth = 500;
    let text = format!("{}1{}", "[".repeat(depth), "]".repeat(depth));
    let mut doc = Document::new(Json, text)?;
    let at = depth as u32;
    let edit = doc.edit(Span::new(at, at + 1), "2")?;
    assert!(!edit.is_full());
    assert_eq!(edit.span(), Span::new(at - 1, at + 2));
    assert_consistent(&doc, Json)
}

#[test]
fn test_to_syntax_matches_tree() -> TestResult {
    let doc = Document::new(Json, r#"{"a": [1, {"b": null}]} // done"#)?;
    let cst = doc.tree().to_syntax();
    assert_eq!(cst.span(), doc.tree().root().span());
    let ours: Vec<(Kind, Span)> = doc
        .tree()
        .root()
        .tokens()
        .map(|t| (t.kind, t.span))
        .collect();
    let theirs: Vec<(Kind, Span)> = cst.tokens().map(|t| (t.kind, t.span)).collect();
    assert_eq!(ours, theirs);
    let ours: Vec<(Kind, Span)> = doc
        .tree()
        .root()
        .descendants()
        .map(|n| (n.kind(), n.span()))
        .collect();
    let theirs: Vec<(Kind, Span)> = cst.descendants().map(|n| (*n.kind(), n.span())).collect();
    assert_eq!(ours, theirs);
    Ok(())
}

#[test]
fn test_trivia_is_placed_outside_nodes() -> TestResult {
    let doc = Document::new(Json, "  [ 1 , /* c */ 2 ]  ")?;
    let array = doc
        .tree()
        .root()
        .descendants()
        .find(|n| n.kind() == Kind::Array)
        .ok_or("no array")?;
    assert_eq!(array.text(doc.text()), Some("[ 1 , /* c */ 2 ]"));
    // Leading and trailing whitespace belong to the root.
    let root_children: Vec<Kind> = doc.tree().root().children().map(Element::kind).collect();
    assert_eq!(
        root_children,
        [Kind::Whitespace, Kind::Array, Kind::Whitespace]
    );
    Ok(())
}

#[test]
fn test_missing_value_marker_sits_after_colon() -> TestResult {
    let doc = Document::new(Json, r#"{"a":   }"#)?;
    let member = doc
        .tree()
        .root()
        .descendants()
        .find(|n| n.kind() == Kind::Member)
        .ok_or("no member")?;
    assert_eq!(member.text(doc.text()), Some(r#""a":"#));
    let error = member
        .children()
        .find_map(Element::as_node)
        .ok_or("no marker")?;
    assert_eq!((error.kind(), error.span()), (Kind::Error, Span::empty(5)));
    Ok(())
}

#[test]
fn test_tree_snapshot_is_send_and_sync() -> TestResult {
    fn assert_send_sync<T: Send + Sync>(_: &T) {}
    let doc = Document::new(Json, "[1, 2]")?;
    assert_send_sync(&doc);
    let snapshot = doc.tree().clone();
    assert_send_sync(&snapshot);
    let handle = std::thread::spawn(move || snapshot.root().tokens().count());
    assert_eq!(handle.join().map_err(|_| "worker panicked")?, 6);
    Ok(())
}

// ---------------------------------------------------------------------------
// Grammars that exercise the document's defences.
// ---------------------------------------------------------------------------

/// JSONC with members reparsable too.
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
        matches!(kind, Kind::Object | Kind::Array | Kind::Member)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tk {
    Root,
    Char,
    Eof,
}

impl TokenKind for Tk {
    fn is_eof(&self) -> bool {
        matches!(self, Tk::Eof)
    }
}

/// A lexer that leaves a gap whenever the text contains `!`, and emits an
/// end-of-input marker otherwise.
struct Gappy;

impl Grammar for Gappy {
    type Kind = Tk;
    const ROOT: Tk = Tk::Root;
    fn lex(&self, text: &str, tokens: &mut Vec<Token<Tk>>) {
        for (i, c) in text.char_indices() {
            if c == '!' {
                continue;
            }
            let end = (i + c.len_utf8()) as u32;
            tokens.push(Token::new(Tk::Char, Span::new(i as u32, end)));
        }
        tokens.push(Token::new(Tk::Eof, Span::empty(text.len() as u32)));
    }
    fn parse(&self, _: Tk, b: &mut Builder<'_, Tk>) {
        b.start(Tk::Root);
        while !b.at_end() {
            b.bump();
        }
        b.finish();
    }
}

#[test]
fn test_end_of_input_markers_are_dropped() -> TestResult {
    let doc = Document::new(Gappy, "ab")?;
    let kinds: Vec<Tk> = doc.tree().root().tokens().map(|t| t.kind).collect();
    assert_eq!(kinds, [Tk::Char, Tk::Char]);
    Ok(())
}

#[test]
fn test_lexer_gap_is_reported_by_new() {
    assert_eq!(
        Document::new(Gappy, "a!b").map(|_| ()),
        Err(Error::Tokens { offset: 1 })
    );
}

#[test]
fn test_lexer_gap_during_edit_rolls_back() -> TestResult {
    let mut doc = Document::new(Gappy, "abc")?;
    let before = doc.tree().clone();
    let stats = doc.stats();
    assert_eq!(
        doc.edit(Span::new(1, 2), "!"),
        Err(Error::Tokens { offset: 1 })
    );
    assert_eq!(doc.text(), "abc");
    assert_eq!(doc.tree(), &before);
    assert_eq!(doc.stats(), stats);
    // The document is still usable.
    doc.edit(Span::new(1, 2), "z")?;
    assert_eq!(doc.text(), "azc");
    Ok(())
}

/// JSONC whose lexer drops its first token whenever the text contains `!`.
#[derive(Clone, Copy)]
struct BrokenJson;

impl Grammar for BrokenJson {
    type Kind = Kind;
    const ROOT: Kind = Kind::Document;
    fn lex(&self, text: &str, tokens: &mut Vec<Token<Kind>>) {
        json::lex(text, tokens);
        if text.contains('!') && !tokens.is_empty() {
            let _first = tokens.remove(0);
        }
    }
    fn parse(&self, kind: Kind, b: &mut Builder<'_, Kind>) {
        json::parse(kind, b);
    }
    fn is_reparsable(&self, kind: Kind) -> bool {
        Json.is_reparsable(kind)
    }
}

#[test]
fn test_refused_edit_leaves_statistics_untouched() -> TestResult {
    let mut doc = Document::new(BrokenJson, "[[1, 2], 3]")?;
    let stats = doc.stats();
    // The inner array's attempt is refused (its window does not tile), then
    // the full reparse fails: the edit is refused as a whole.
    assert_eq!(
        doc.edit(Span::new(3, 3), "!"),
        Err(Error::Tokens { offset: 0 })
    );
    assert_eq!(doc.stats(), stats);
    assert_eq!(doc.text(), "[[1, 2], 3]");
    Ok(())
}

/// A grammar that builds nothing at all: the document must still produce a
/// lossless root.
struct Lazy;

impl Grammar for Lazy {
    type Kind = Tk;
    const ROOT: Tk = Tk::Root;
    fn lex(&self, text: &str, tokens: &mut Vec<Token<Tk>>) {
        Gappy.lex(text, tokens);
    }
    fn parse(&self, _: Tk, _: &mut Builder<'_, Tk>) {}
}

#[test]
fn test_grammar_that_builds_nothing_still_yields_lossless_root() -> TestResult {
    let mut doc = Document::new(Lazy, "xyz")?;
    assert_eq!(doc.tree().root().kind(), Tk::Root);
    assert_eq!(doc.tree().root().tokens().count(), 3);
    doc.edit(Span::new(3, 3), "w")?;
    assert_eq!(doc.tree().root().span(), Span::new(0, 4));
    Ok(())
}

#[test]
fn test_edit_is_transactional_when_grammar_panics() -> TestResult {
    use std::panic::{AssertUnwindSafe, catch_unwind};

    /// JSONC whose lexer panics on `!` — a stand-in for a bug in grammar code.
    #[derive(Clone, Copy)]
    struct Panicky;
    impl Grammar for Panicky {
        type Kind = Kind;
        const ROOT: Kind = Kind::Document;
        fn lex(&self, text: &str, tokens: &mut Vec<Token<Kind>>) {
            assert!(!text.contains('!'), "grammar bug triggered");
            json::lex(text, tokens);
        }
        fn parse(&self, kind: Kind, b: &mut Builder<'_, Kind>) {
            json::parse(kind, b);
        }
        fn is_reparsable(&self, kind: Kind) -> bool {
            Json.is_reparsable(kind)
        }
    }

    let mut doc = Document::new(Panicky, "[[1, 2], 3]")?;
    let before = doc.tree().clone();
    let stats = doc.stats();
    // The panic happens while relexing the inner array for a partial attempt.
    let outcome = catch_unwind(AssertUnwindSafe(|| doc.edit(Span::new(3, 3), "!")));
    assert!(outcome.is_err(), "the grammar was expected to panic");
    // Nothing was committed: text, tree, and every counter are as they were.
    assert_eq!(doc.text(), "[[1, 2], 3]");
    assert_eq!(doc.tree(), &before);
    assert_eq!(doc.stats(), stats);
    // And the document still works.
    doc.edit(Span::new(3, 3), "0")?;
    assert_eq!(doc.text(), "[[10, 2], 3]");
    assert_consistent(&doc, Panicky)
}

#[test]
fn test_edit_beneath_a_deep_chain_of_plain_nodes() -> TestResult {
    // `[[x]+x+x+...]`: a left-associative chain thousands of sums deep, none
    // of them reparsable, with a small list at the bottom. An edit inside that
    // list is served by it alone.
    let depth = 3_000;
    let mut text = String::from("[[x]");
    for _ in 0..depth {
        text.push_str("+x");
    }
    text.push(']');
    let mut doc = Document::new(Chain, text)?;
    let edit = doc.edit(Span::new(2, 3), "y")?;
    assert!(!edit.is_full());
    assert_eq!((edit.kind(), edit.span()), (Ck::List, Span::new(1, 4)));
    assert_eq!(doc.tree(), Document::new(Chain, doc.text())?.tree());
    Ok(())
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

/// `[a+b+c]` lists of left-associative sums, built with checkpoints.
#[derive(Clone, Copy)]
struct Chain;

impl Grammar for Chain {
    type Kind = Ck;
    const ROOT: Ck = Ck::Root;
    fn lex(&self, text: &str, tokens: &mut Vec<Token<Ck>>) {
        for (i, c) in text.char_indices() {
            let kind = match c {
                '[' => Ck::Open,
                ']' => Ck::Close,
                '+' => Ck::Plus,
                _ => Ck::Word,
            };
            let end = (i + c.len_utf8()) as u32;
            tokens.push(Token::new(kind, Span::new(i as u32, end)));
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
    b.bump(); // `[`
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

#[test]
fn test_error_display_is_descriptive() {
    let e = Error::NotCharBoundary { offset: 3 };
    assert_eq!(
        e.to_string(),
        "edit boundary at byte 3 falls inside a UTF-8 character"
    );
    let boxed: Box<dyn std::error::Error> = Box::new(e);
    assert!(boxed.source().is_none());
}
