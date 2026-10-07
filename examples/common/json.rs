//! A grammar for JSON with comments (JSONC), written against incremental-lang.
//!
//! It is shared by the examples, the integration and property tests, and the
//! benchmarks, and it doubles as a template for wiring a real language into
//! the crate:
//!
//! - one `Kind` enum names both nodes and tokens;
//! - [`lex`] is a hand-written longest-match lexer that tiles its input;
//! - [`parse`] is recursive descent with error recovery — malformed input
//!   becomes `Error` nodes, never a failure;
//! - objects and arrays are reparsable, because they are delimited by brackets
//!   that cannot merge with neighbouring tokens.

use incremental_lang::{Builder, Grammar, Span, Token, TokenKind};

/// Node and token kinds of JSONC.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    // Nodes
    /// The whole file.
    Document,
    /// `{ ... }`
    Object,
    /// `[ ... ]`
    Array,
    /// `"key": value` inside an object.
    Member,
    /// Something malformed: a stray token, or (when empty) something missing.
    Error,

    // Tokens
    /// `{`
    LBrace,
    /// `}`
    RBrace,
    /// `[`
    LBracket,
    /// `]`
    RBracket,
    /// `:`
    Colon,
    /// `,`
    Comma,
    /// A terminated string literal.
    String,
    /// A number literal.
    Number,
    /// `true`
    True,
    /// `false`
    False,
    /// `null`
    Null,
    /// Spaces, tabs, and line breaks.
    Whitespace,
    /// `// ...` or `/* ... */`.
    Comment,
    /// Anything else: an unterminated string, an unknown word, a stray byte.
    Unknown,
}

impl TokenKind for Kind {
    fn is_trivia(&self) -> bool {
        matches!(self, Kind::Whitespace | Kind::Comment)
    }
}

/// The JSONC grammar, with objects and arrays reparsable.
#[derive(Clone, Copy, Debug, Default)]
pub struct Json;

impl Grammar for Json {
    type Kind = Kind;
    const ROOT: Kind = Kind::Document;

    fn lex(&self, text: &str, tokens: &mut Vec<Token<Kind>>) {
        lex(text, tokens);
    }

    fn parse(&self, kind: Kind, builder: &mut Builder<'_, Kind>) {
        parse(kind, builder);
    }

    fn is_reparsable(&self, kind: Kind) -> bool {
        matches!(kind, Kind::Object | Kind::Array)
    }
}

/// Lexes JSONC. Every token ends on a UTF-8 boundary, and the tokens tile the
/// text exactly.
pub fn lex(text: &str, tokens: &mut Vec<Token<Kind>>) {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        let kind = match bytes[i] {
            b'{' => single(&mut i, Kind::LBrace),
            b'}' => single(&mut i, Kind::RBrace),
            b'[' => single(&mut i, Kind::LBracket),
            b']' => single(&mut i, Kind::RBracket),
            b':' => single(&mut i, Kind::Colon),
            b',' => single(&mut i, Kind::Comma),
            b' ' | b'\t' | b'\r' | b'\n' => {
                i = run(bytes, i, |b| matches!(b, b' ' | b'\t' | b'\r' | b'\n'));
                Kind::Whitespace
            }
            b'"' => string(bytes, &mut i),
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                i = run(bytes, i, |b| b != b'\n');
                Kind::Comment
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = block_comment_end(bytes, i);
                Kind::Comment
            }
            b'-' | b'0'..=b'9' => {
                i = run(bytes, i + 1, |b| {
                    b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-')
                });
                Kind::Number
            }
            b if b.is_ascii_alphabetic() => {
                i = run(bytes, i, |b| b.is_ascii_alphanumeric() || b == b'_');
                match &text[start..i] {
                    "true" => Kind::True,
                    "false" => Kind::False,
                    "null" => Kind::Null,
                    _ => Kind::Unknown,
                }
            }
            _ => {
                // One character, however many bytes it takes.
                let width = text[i..].chars().next().map_or(1, char::len_utf8);
                i += width;
                Kind::Unknown
            }
        };
        tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
    }
}

fn single(i: &mut usize, kind: Kind) -> Kind {
    *i += 1;
    kind
}

/// Advances past every byte from `i` that satisfies `keep`.
fn run(bytes: &[u8], mut i: usize, keep: impl Fn(u8) -> bool) -> usize {
    while i < bytes.len() && keep(bytes[i]) {
        i += 1;
    }
    i
}

/// Lexes a string starting at the opening quote. A string ends at its closing
/// quote; one that reaches a line break or the end first is unterminated.
fn string(bytes: &[u8], i: &mut usize) -> Kind {
    let mut j = *i + 1;
    while j < bytes.len() {
        match bytes[j] {
            b'"' => {
                *i = j + 1;
                return Kind::String;
            }
            b'\\' if j + 1 < bytes.len() && bytes[j + 1] != b'\n' => j += 2,
            b'\n' => break,
            _ => j += 1,
        }
    }
    *i = j;
    Kind::Unknown
}

/// The end of a block comment starting at `i`, or the end of input if it is
/// unterminated.
fn block_comment_end(bytes: &[u8], i: usize) -> usize {
    let mut j = i + 2;
    while j + 1 < bytes.len() {
        if bytes[j] == b'*' && bytes[j + 1] == b'/' {
            return j + 2;
        }
        j += 1;
    }
    bytes.len()
}

/// Parses a node of `kind`: the whole document, or one object, array, or
/// member on its own.
pub fn parse(kind: Kind, b: &mut Builder<'_, Kind>) {
    match kind {
        Kind::Object => object(b),
        Kind::Array => array(b),
        Kind::Member => member(b),
        _ => document(b),
    }
}

fn document(b: &mut Builder<'_, Kind>) {
    b.start(Kind::Document);
    while !b.at_end() {
        match b.peek() {
            // A closer, separator, or stray byte at the top level: wrap it in
            // an error node and move on.
            Some(Kind::RBrace | Kind::RBracket | Kind::Comma | Kind::Colon) => stray(b),
            _ => value(b),
        }
    }
    b.finish();
}

fn value(b: &mut Builder<'_, Kind>) {
    match b.peek() {
        Some(Kind::LBrace) => object(b),
        Some(Kind::LBracket) => array(b),
        Some(Kind::String | Kind::Number | Kind::True | Kind::False | Kind::Null) => b.bump(),
        Some(Kind::Unknown) => stray(b),
        // A closer, a separator, or the end: the value is missing. Leave the
        // token for the enclosing construct.
        _ => missing(b),
    }
}

fn object(b: &mut Builder<'_, Kind>) {
    b.start(Kind::Object);
    b.bump(); // `{`
    // Closers are tested before the end, so a balanced object never looks past
    // its own `}` — the condition for reparsing it on its own.
    while !at_closer(b) && !b.at_end() {
        member(b);
        skip_junk(b);
        if !b.eat(Kind::Comma) {
            break;
        }
    }
    expect(b, Kind::RBrace);
    b.finish();
}

fn member(b: &mut Builder<'_, Kind>) {
    b.start(Kind::Member);
    expect(b, Kind::String);
    expect(b, Kind::Colon);
    value(b);
    b.finish();
}

fn array(b: &mut Builder<'_, Kind>) {
    b.start(Kind::Array);
    b.bump(); // `[`
    while !at_closer(b) && !b.at_end() {
        value(b);
        skip_junk(b);
        if !b.eat(Kind::Comma) {
            break;
        }
    }
    expect(b, Kind::RBracket);
    b.finish();
}

/// Whether the next token closes an object or an array. Either kind of closer
/// ends a list: a mismatched one is left for the enclosing construct.
fn at_closer(b: &Builder<'_, Kind>) -> bool {
    b.at(Kind::RBrace) || b.at(Kind::RBracket)
}

/// Error recovery inside a list: wraps everything up to the next `,` or
/// closer in `Error` nodes, so one malformed element does not end the list.
/// A bracketed value among the junk is parsed whole, which keeps its brackets
/// paired — a stray `[` must not let its `]` close the enclosing list.
fn skip_junk(b: &mut Builder<'_, Kind>) {
    while !b.at(Kind::Comma) && !at_closer(b) && !b.at_end() {
        b.start(Kind::Error);
        if b.at(Kind::LBrace) || b.at(Kind::LBracket) {
            value(b);
        } else {
            b.bump();
        }
        b.finish();
    }
}

/// Consumes a token of `kind`, or records that it is missing.
fn expect(b: &mut Builder<'_, Kind>, kind: Kind) {
    if !b.eat(kind) {
        missing(b);
    }
}

/// An empty `Error` node: something required is absent here.
fn missing(b: &mut Builder<'_, Kind>) {
    b.start(Kind::Error);
    b.finish();
}

/// An `Error` node around one unexpected token.
fn stray(b: &mut Builder<'_, Kind>) {
    b.start(Kind::Error);
    b.bump();
    b.finish();
}
