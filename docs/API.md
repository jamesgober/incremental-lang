# incremental-lang &mdash; API Reference

> Complete reference for every public item in `incremental-lang`, with examples.
> **Status: stable (1.0).** The surface below is the `1.0` contract; it follows
> [Semantic Versioning](#stability) and will not change in a breaking way before
> `2.0`. See [`../dev/ROADMAP.md`](../dev/ROADMAP.md).

<sub>Copyright &copy; 2026 <strong>James Gober</strong>.</sub>

## Table of contents

- [Overview](#overview)
- [Installation](#installation)
- [Quick start](#quick-start)
- [Concepts](#concepts)
  - [Lossless trees and kinds](#lossless-trees-and-kinds)
  - [Widths, not positions](#widths-not-positions)
  - [What happens on an edit](#what-happens-on-an-edit)
  - [When a single node is reparsed](#when-a-single-node-is-reparsed)
  - [Trivia placement](#trivia-placement)
  - [Errors live in the tree](#errors-live-in-the-tree)
- [`Grammar`](#grammar)
  - [`Grammar::Kind`](#grammarkind)
  - [`Grammar::ROOT`](#grammarroot)
  - [`Grammar::lex`](#grammarlex)
  - [`Grammar::parse`](#grammarparse)
  - [`Grammar::is_reparsable`](#grammaris_reparsable)
- [`Builder`](#builder)
  - [`Builder::peek`](#builderpeek)
  - [`Builder::nth`](#buildernth)
  - [`Builder::at`](#builderat)
  - [`Builder::at_end`](#builderat_end)
  - [`Builder::bump`](#builderbump)
  - [`Builder::eat`](#buildereat)
  - [`Builder::start`](#builderstart)
  - [`Builder::finish`](#builderfinish)
  - [`Builder::checkpoint`](#buildercheckpoint)
  - [`Builder::start_at`](#builderstart_at)
- [`Checkpoint`](#checkpoint)
- [`Document`](#document)
  - [`Document::new`](#documentnew)
  - [`Document::edit`](#documentedit)
  - [`Document::text`](#documenttext)
  - [`Document::tree`](#documenttree)
  - [`Document::grammar`](#documentgrammar)
  - [`Document::stats`](#documentstats)
- [`Reparse`](#reparse)
- [`Stats`](#stats)
- [`Tree`](#tree)
  - [`Tree::root`](#treeroot)
  - [`Tree::to_syntax`](#treeto_syntax)
- [`Node`](#node)
  - [`Node::kind`](#nodekind)
  - [`Node::span`](#nodespan)
  - [`Node::text`](#nodetext)
  - [`Node::children`](#nodechildren)
  - [`Node::descendants`](#nodedescendants)
  - [`Node::tokens`](#nodetokens)
- [`Element`](#element)
- [`Error`](#error)
- [Re-exports: `Span`, `Token`, `TokenKind`](#re-exports-span-token-tokenkind)
- [Feature flags](#feature-flags)
- [Guide: writing a grammar](#guide-writing-a-grammar)
- [Guide: wiring a document into an editor](#guide-wiring-a-document-into-an-editor)
- [Stability](#stability)

---

## Overview

incremental-lang keeps a source text and its lossless concrete syntax tree in
step under edits, rebuilding only the part of the tree around each change. A
language supplies a [`Grammar`](#grammar); a [`Document`](#document) does the
lexing, parsing, incremental reparsing, and the checks that keep the result
identical to a fresh parse.

| Item | Role |
|---|---|
| [`Grammar`](#grammar) | What a language supplies: kinds, lexer, parser, reparsable kinds. |
| [`Builder`](#builder) | The cursor and tree builder a grammar's parser works with. |
| [`Checkpoint`](#checkpoint) | A position for opening a node around children already built. |
| [`Document`](#document) | The text and its tree; applies edits. |
| [`Reparse`](#reparse) | What one edit rebuilt. |
| [`Stats`](#stats) | Running counters of a document's work. |
| [`Tree`](#tree) | An immutable, cheaply cloned, thread-safe snapshot of the syntax tree. |
| [`Node`](#node) | A positioned view of one node of a tree. |
| [`Element`](#element) | One child of a node: a node or a token. |
| [`Error`](#error) | Why a document could not be created or an edit was refused. |
| [`Span`, `Token`, `TokenKind`](#re-exports-span-token-tokenkind) | Re-exported from `syntax-lang`. |

The crate is `#![forbid(unsafe_code)]`, `no_std`-compatible (needs only
`alloc`), and depends on [`syntax-lang`](https://crates.io/crates/syntax-lang)
alone.

---

## Installation

```toml
[dependencies]
incremental-lang = "1"
```

Or from the terminal:

```bash
cargo add incremental-lang
```

MSRV: Rust 1.85 (Rust 2024 edition).

---

## Quick start

Most examples in this reference use the grammar below — nested lists such as
`(a (b c))`, with lists reparsable — and hide it to stay short. It is complete
here:

```rust
use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum K { Root, List, Open, Close, Atom, Space }

impl TokenKind for K {
    fn is_trivia(&self) -> bool { matches!(self, K::Space) }
}

struct Lists;

impl Grammar for Lists {
    type Kind = K;
    const ROOT: K = K::Root;

    fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
        let bytes = text.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let start = i;
            let kind = match bytes[i] {
                b'(' => { i += 1; K::Open }
                b')' => { i += 1; K::Close }
                b' ' => {
                    while i < bytes.len() && bytes[i] == b' ' { i += 1; }
                    K::Space
                }
                _ => {
                    while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; }
                    K::Atom
                }
            };
            tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
        }
    }

    fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
        if kind == K::List {
            return list(b);
        }
        b.start(K::Root);
        while !b.at_end() {
            if b.at(K::Open) { list(b) } else { b.bump() }
        }
        b.finish();
    }

    fn is_reparsable(&self, kind: K) -> bool {
        kind == K::List
    }
}

fn list(b: &mut Builder<'_, K>) {
    b.start(K::List);
    b.bump(); // `(`
    while !b.at(K::Close) && !b.at_end() {
        if b.at(K::Open) { list(b) } else { b.bump() }
    }
    b.eat(K::Close);
    b.finish();
}

let mut doc = Document::new(Lists, "(let (x 1) (y 2))")?;
let edit = doc.edit(Span::new(8, 9), "10")?;
assert!(!edit.is_full());
assert_eq!(edit.span(), Span::new(5, 11)); // only `(x 10)` was rebuilt
assert_eq!(doc.tree(), Document::new(Lists, doc.text())?.tree());
# Ok::<(), incremental_lang::Error>(())
```

---

## Concepts

### Lossless trees and kinds

The tree is a *concrete* syntax tree: every byte of the source belongs to
exactly one token, and whitespace and comments (trivia) are tokens like any
other. Concatenating the tokens of any node reproduces its text, and the root
reproduces the document. One kind type names both nodes and tokens — the model
`syntax-lang` and `rowan` use — so a language defines a single `enum` with its
composite kinds (`Object`, `List`) and its lexical kinds (`String`, `Space`).

### Widths, not positions

Internally, a node records how many bytes it covers and what its children
are, but not where it starts. After an edit, everything before the change is
where it was, and everything after it has moved — but a subtree that stores
only widths is still correct at its new offset, so it is reused without being
touched. Positions are computed on the way down by the [`Node`](#node) view,
which pairs a reference into the tree with an absolute offset.

### What happens on an edit

[`Document::edit`](#documentedit) replaces a byte range of the text and then:

1. **Locates** the innermost node that strictly contains the edited range —
   begins before it and ends after it — noting every node on the path from the
   root.
2. **Tries** each [reparsable](#grammaris_reparsable) node on that path, from
   the innermost outwards: relexes its new text together with the token on
   each side of it, runs the [checks](#when-a-single-node-is-reparsed), and
   asks the grammar to [`parse`](#grammarparse) a node of that kind from the
   node's tokens alone.
3. **Splices** the first node that passes into the tree, adjusting the widths
   on the path above it. When the document is the only owner of its tree the
   path is updated in place; while a snapshot shares it, the path is copied.
4. **Falls back** to a whole-document parse when no candidate passes, or when
   trying the next candidate would push the bytes lexed on failed attempts
   past the length of the document. An edit therefore never costs more than
   about two full parses.

### When a single node is reparsed

A node is replaced by its own reparse only if all of these hold:

- the edit lies strictly inside it, so its first and last bytes are untouched;
- relexing it together with its neighbouring tokens reproduces both
  neighbours exactly — given the lexer contract, no token grew across the
  node's boundary;
- its new first and last tokens have the same kinds as before;
- the grammar built exactly one node, of the same kind, containing every token,
  and never looked past the node's last token.

The last check is what makes the scheme generic. A list whose `)` was deleted
does not know it is unbalanced — it simply keeps asking for tokens, runs off
the end of its text, and is refused. So is an object that an unterminated
string or comment has eaten into. No table of bracket pairs is needed.

### Trivia placement

The grammar never sees trivia; the [`Builder`](#builder) places it. Trivia
between two tokens of a node goes inside that node. Trivia before a node's
first token goes to the *enclosing* node, ahead of it. So every node that holds
a token begins and ends on a significant token, and a node's span is exactly
its syntax — which is what lets a node's text be relexed on its own. A node
with no tokens (a marker for something missing) sits right after the last
token before it. Trivia at the very start and end of the document belongs to
the root.

### Errors live in the tree

A grammar records syntax errors as nodes of its own kind type: an error node
around a token that does not belong, an empty one where something is missing.
Because the errors are part of the tree, they move with it — an incremental
edit updates them along with everything else, and there is no separate list of
diagnostics with positions to keep in step. Walk the tree to collect them.

---

## `Grammar`

```rust,ignore
pub trait Grammar {
    type Kind: TokenKind + Copy + Eq;
    const ROOT: Self::Kind;
    fn lex(&self, text: &str, tokens: &mut Vec<Token<Self::Kind>>);
    fn parse(&self, kind: Self::Kind, builder: &mut Builder<'_, Self::Kind>);
    fn is_reparsable(&self, kind: Self::Kind) -> bool { false }
}
```

A language: its kinds, its lexer, its parser, and which node kinds may be
reparsed on their own. A grammar that marks nothing reparsable is still
correct — every edit is then a full reparse — so incremental reparsing is
something a grammar opts into kind by kind.

**The lexer contract.** [`lex`](#grammarlex) must *tile* its input: tokens
start at byte 0, each starts where the previous ended, the last ends at the
end of the text. Zero-width tokens are allowed; zero-width end-of-input markers
are dropped. For single-node reparsing the lexer must also be:

- **Restartable** — started at a token boundary, it produces the tokens a full
  lex would produce from there. A lexer whose output depends on text far
  behind (an indentation stack, a heredoc terminator) is not.
- **Limited to one character of lookahead** — where a token ends may depend on
  at most one character after it. A lexer that scans ahead and backs off (try
  `"…"` to a closing quote, fall back to a lone `"` when there is none) looks
  arbitrarily far: a quote typed later on the line would change how earlier
  text lexes. Let unterminated literals run to the end of the line or input
  instead.

**The parser contract.** For a reparsable kind:

- **The node stands alone** — [`parse`](#grammarparse) makes the same
  decisions the whole-document parse makes at that point; nothing about how
  the node is parsed depends on what surrounds it.
- **Nothing around it looks inside** — outside the node, no decision depends
  on its contents beyond its first token: the decision to start it rests on
  that token alone, no lookahead reaches past it into the node, and nothing
  its parse computes feeds back into the parse around it.

The document checks what can be checked cheaply (see
[When a single node is reparsed](#when-a-single-node-is-reparsed)). Lexer
lookahead and outside code depending on a node's contents cannot be seen from
a single reparse, so a grammar that breaks those two rules can end up with a
tree that differs from a fresh parse. Hold a grammar's incremental trees to
fresh parses in a property test to be sure.

**Choosing reparsable kinds.** Mark the constructs that begin and end with
tokens that never merge with their neighbours: blocks, objects, arrays,
argument lists. Most typing happens inside them, and their extent is fixed by
their brackets.

### `Grammar::Kind`

```rust,ignore
type Kind: TokenKind + Copy + Eq;
```

The kind type shared by nodes and tokens. [`TokenKind`](#re-exports-span-token-tokenkind)
marks trivia (`is_trivia`) and end-of-input markers (`is_eof`); both default
to `false`. Kinds are copied and compared constantly, so keep them small — a
fieldless `enum` is typical.

```rust
use incremental_lang::TokenKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    // nodes
    Document, Block, Statement,
    // tokens
    LBrace, RBrace, Ident, Semi, Whitespace, LineComment,
}

impl TokenKind for Kind {
    fn is_trivia(&self) -> bool {
        matches!(self, Kind::Whitespace | Kind::LineComment)
    }
}

assert!(Kind::LineComment.is_trivia());
assert!(!Kind::Ident.is_trivia());
```

A kind may carry data, as long as it stays `Copy + Eq` — for instance an error
kind that records what was expected:

```rust
use incremental_lang::TokenKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Expected { Value, Colon, Close }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind { Root, Missing(Expected), Word, Space }

impl TokenKind for Kind {
    fn is_trivia(&self) -> bool { matches!(self, Kind::Space) }
}

assert_ne!(Kind::Missing(Expected::Colon), Kind::Missing(Expected::Close));
```

### `Grammar::ROOT`

```rust,ignore
const ROOT: Self::Kind;
```

The kind of the node spanning the whole document. Every whole-document parse
calls [`parse`](#grammarparse) with it. The document guarantees the tree's root
has this kind: if the grammar builds exactly one top-level node of this kind,
the trivia around it is moved inside it; otherwise everything is wrapped in a
new root.

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, List, Open, Close, Atom, Space }
# impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
# struct Lists;
# impl Grammar for Lists {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         let bytes = text.as_bytes();
#         let mut i = 0;
#         while i < bytes.len() {
#             let start = i;
#             let kind = match bytes[i] {
#                 b'(' => { i += 1; K::Open }
#                 b')' => { i += 1; K::Close }
#                 b' ' => { while i < bytes.len() && bytes[i] == b' ' { i += 1; } K::Space }
#                 _ => { while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; } K::Atom }
#             };
#             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
#         }
#     }
#     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
#         if kind == K::List { return list(b); }
#         b.start(K::Root);
#         while !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#         b.finish();
#     }
#     fn is_reparsable(&self, kind: K) -> bool { kind == K::List }
# }
# fn list(b: &mut Builder<'_, K>) {
#     b.start(K::List);
#     b.bump();
#     while !b.at(K::Close) && !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#     b.eat(K::Close);
#     b.finish();
# }
let doc = Document::new(Lists, "  (a)  ")?;
let root = doc.tree().root();
assert_eq!(root.kind(), Lists::ROOT);
// Leading and trailing spaces belong to the root.
assert_eq!(root.span(), Span::new(0, 7));
assert_eq!(root.children().count(), 3); // Space, List, Space
# Ok::<(), incremental_lang::Error>(())
```

A grammar that builds nothing still yields a lossless root:

```rust
use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum K { Root, Char }
impl TokenKind for K {}

struct Idle;
impl Grammar for Idle {
    type Kind = K;
    const ROOT: K = K::Root;
    fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
        for (i, c) in text.char_indices() {
            let end = (i + c.len_utf8()) as u32;
            tokens.push(Token::new(K::Char, Span::new(i as u32, end)));
        }
    }
    fn parse(&self, _: K, _: &mut Builder<'_, K>) {}
}

let doc = Document::new(Idle, "abc")?;
assert_eq!(doc.tree().root().kind(), K::Root);
assert_eq!(doc.tree().root().tokens().count(), 3);
# Ok::<(), incremental_lang::Error>(())
```

### `Grammar::lex`

```rust,ignore
fn lex(&self, text: &str, tokens: &mut Vec<Token<Self::Kind>>);
```

Lexes `text`, appending tokens to `tokens`.

| Parameter | Meaning |
|---|---|
| `text` | The text to lex: the whole document, or a slice around a node being relexed. Spans are relative to its start. |
| `tokens` | Arrives empty; the lexer appends to it. The buffer is reused across edits, so lexing allocates only while it grows. |

The tokens must tile `text` exactly; a lexer that leaves a gap or overlap is
refused with [`Error::Tokens`](#error) rather than trusted. Keep tokens on
UTF-8 boundaries.

```rust
use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum K { Root, Word, Space }
impl TokenKind for K {
    fn is_trivia(&self) -> bool { matches!(self, K::Space) }
}

struct Words;
impl Grammar for Words {
    type Kind = K;
    const ROOT: K = K::Root;
    fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
        // Runs of spaces and runs of everything else. Walking characters
        // keeps every boundary on a UTF-8 boundary.
        let class = |c: char| if c == ' ' { K::Space } else { K::Word };
        let mut chars = text.char_indices().peekable();
        while let Some((start, c)) = chars.next() {
            let kind = class(c);
            let mut end = start + c.len_utf8();
            while let Some(&(i, next)) = chars.peek() {
                if class(next) != kind {
                    break;
                }
                end = i + next.len_utf8();
                chars.next();
            }
            tokens.push(Token::new(kind, Span::new(start as u32, end as u32)));
        }
    }
    fn parse(&self, _: K, b: &mut Builder<'_, K>) {
        b.start(K::Root);
        while !b.at_end() { b.bump(); }
        b.finish();
    }
}

let doc = Document::new(Words, "héllo  wörld")?;
let tokens: Vec<(K, &str)> = doc
    .tree()
    .root()
    .tokens()
    .map(|t| (t.kind, &doc.text()[t.span.start().to_usize()..t.span.end().to_usize()]))
    .collect();
assert_eq!(tokens, [(K::Word, "héllo"), (K::Space, "  "), (K::Word, "wörld")]);
# Ok::<(), incremental_lang::Error>(())
```

A lexer that does not tile its input is reported:

```rust
use incremental_lang::{Builder, Document, Error, Grammar, Span, Token, TokenKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum K { Root, Char }
impl TokenKind for K {}

// Skips every `!`, leaving a gap.
struct Skips;
impl Grammar for Skips {
    type Kind = K;
    const ROOT: K = K::Root;
    fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
        for (i, c) in text.char_indices().filter(|&(_, c)| c != '!') {
            let end = (i + c.len_utf8()) as u32;
            tokens.push(Token::new(K::Char, Span::new(i as u32, end)));
        }
    }
    fn parse(&self, _: K, _: &mut Builder<'_, K>) {}
}

assert_eq!(Document::new(Skips, "a!b").err(), Some(Error::Tokens { offset: 1 }));
```

### `Grammar::parse`

```rust,ignore
fn parse(&self, kind: Self::Kind, builder: &mut Builder<'_, Self::Kind>);
```

Parses one node of kind `kind` from the tokens `builder` offers.

| Parameter | Meaning |
|---|---|
| `kind` | [`ROOT`](#grammarroot) for a whole-document parse; a [reparsable](#grammaris_reparsable) kind to reparse a single node, in which case the builder offers exactly that node's tokens. |
| `builder` | The cursor and tree builder; see [`Builder`](#builder). |

The usual shape is a dispatch on `kind` to the function that parses that
construct — the same function the whole-document parse calls when it meets
one, which is how the grammar keeps its single-node parses consistent with its
full parse. Kinds that are neither the root nor reparsable never reach it.

```rust
use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum K { File, Block, LBrace, RBrace, Word, Space }
impl TokenKind for K {
    fn is_trivia(&self) -> bool { matches!(self, K::Space) }
}

struct Blocks;
impl Grammar for Blocks {
    type Kind = K;
    const ROOT: K = K::File;
    fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
        for (i, c) in text.char_indices() {
            let kind = match c { '{' => K::LBrace, '}' => K::RBrace, ' ' => K::Space, _ => K::Word };
            let end = (i + c.len_utf8()) as u32;
            tokens.push(Token::new(kind, Span::new(i as u32, end)));
        }
    }
    fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
        match kind {
            K::Block => block(b),
            _ => {
                b.start(K::File);
                while !b.at_end() {
                    if b.at(K::LBrace) { block(b) } else { b.bump() }
                }
                b.finish();
            }
        }
    }
    fn is_reparsable(&self, kind: K) -> bool { kind == K::Block }
}

// Called both from the file loop and, on its own, to reparse one block.
fn block(b: &mut Builder<'_, K>) {
    b.start(K::Block);
    b.bump(); // `{`
    while !b.at(K::RBrace) && !b.at_end() {
        if b.at(K::LBrace) { block(b) } else { b.bump() }
    }
    b.eat(K::RBrace);
    b.finish();
}

let mut doc = Document::new(Blocks, "a {b {c}} d")?;
let edit = doc.edit(Span::new(6, 7), "x")?;
assert_eq!((edit.kind(), edit.span()), (K::Block, Span::new(5, 8)));
# Ok::<(), incremental_lang::Error>(())
```

Recovering from errors by recording them as nodes keeps the tree lossless and
the reparse local:

```rust
use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum K { Root, Pair, Error, Key, Eq, Value }
impl TokenKind for K {}

// `k=v` pairs; anything else becomes an Error node.
struct Pairs;
impl Grammar for Pairs {
    type Kind = K;
    const ROOT: K = K::Root;
    fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
        for (i, c) in text.char_indices() {
            let kind = match c { '=' => K::Eq, 'a'..='m' => K::Key, _ => K::Value };
            let end = (i + c.len_utf8()) as u32;
            tokens.push(Token::new(kind, Span::new(i as u32, end)));
        }
    }
    fn parse(&self, _: K, b: &mut Builder<'_, K>) {
        b.start(K::Root);
        while !b.at_end() {
            if b.at(K::Key) && b.nth(1) == Some(K::Eq) {
                b.start(K::Pair);
                b.bump();
                b.bump();
                if !b.eat(K::Value) {
                    b.start(K::Error); // empty: the value is missing
                    b.finish();
                }
                b.finish();
            } else {
                b.start(K::Error);
                b.bump();
                b.finish();
            }
        }
        b.finish();
    }
}

let doc = Document::new(Pairs, "a=xb=c")?;
let errors: Vec<Span> = doc
    .tree()
    .root()
    .descendants()
    .filter(|n| n.kind() == K::Error)
    .map(|n| n.span())
    .collect();
assert_eq!(errors, [Span::empty(5), Span::new(5, 6)]); // `b=` lacks a value; `c` is stray
# Ok::<(), incremental_lang::Error>(())
```

### `Grammar::is_reparsable`

```rust,ignore
fn is_reparsable(&self, kind: Self::Kind) -> bool { false }
```

Whether a node of kind `kind` may be reparsed on its own. The default marks
nothing — always correct, never incremental.

| Parameter | Meaning |
|---|---|
| `kind` | A node kind found on the path to an edit. |

Marking a kind costs nothing when it does not apply: the document's checks
refuse any single-node reparse that would disagree with a fresh parse. But a
kind whose reparses are usually refused wastes the lexing spent on each
attempt — watch [`Stats::rejected`](#stats).

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, List, Open, Close, Atom, Space }
# impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
# struct Lists;
# impl Grammar for Lists {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         let bytes = text.as_bytes();
#         let mut i = 0;
#         while i < bytes.len() {
#             let start = i;
#             let kind = match bytes[i] {
#                 b'(' => { i += 1; K::Open }
#                 b')' => { i += 1; K::Close }
#                 b' ' => { while i < bytes.len() && bytes[i] == b' ' { i += 1; } K::Space }
#                 _ => { while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; } K::Atom }
#             };
#             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
#         }
#     }
#     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
#         if kind == K::List { return list(b); }
#         b.start(K::Root);
#         while !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#         b.finish();
#     }
#     fn is_reparsable(&self, kind: K) -> bool { kind == K::List }
# }
# fn list(b: &mut Builder<'_, K>) {
#     b.start(K::List);
#     b.bump();
#     while !b.at(K::Close) && !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#     b.eat(K::Close);
#     b.finish();
# }
assert!(Lists.is_reparsable(K::List));
assert!(!Lists.is_reparsable(K::Root));
# Ok::<(), incremental_lang::Error>(())
```

With nothing reparsable, every edit is a full reparse:

```rust
use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum K { Root, Char }
impl TokenKind for K {}

struct Plain;
impl Grammar for Plain {
    type Kind = K;
    const ROOT: K = K::Root;
    fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
        for (i, c) in text.char_indices() {
            let end = (i + c.len_utf8()) as u32;
            tokens.push(Token::new(K::Char, Span::new(i as u32, end)));
        }
    }
    fn parse(&self, _: K, b: &mut Builder<'_, K>) {
        b.start(K::Root);
        while !b.at_end() { b.bump(); }
        b.finish();
    }
}

let mut doc = Document::new(Plain, "abc")?;
assert!(doc.edit(Span::new(1, 2), "x")?.is_full());
# Ok::<(), incremental_lang::Error>(())
```

---

## `Builder`

```rust,ignore
pub struct Builder<'a, K> { /* private */ }
```

The cursor and tree builder a grammar parses with. A grammar never creates
one: the document passes it to [`Grammar::parse`](#grammarparse), positioned
over the tokens to parse.

- **Reading** — [`peek`](#builderpeek), [`nth`](#buildernth),
  [`at`](#builderat), and [`at_end`](#builderat_end) look at *significant*
  tokens; trivia and end-of-input markers are invisible.
- **Consuming** — [`bump`](#builderbump) and [`eat`](#buildereat) move a token
  into the node being built.
- **Structure** — [`start`](#builderstart) and [`finish`](#builderfinish) open
  and close nodes; [`checkpoint`](#buildercheckpoint) and
  [`start_at`](#builderstart_at) open a node around children already built.

The tree is lossless whatever the grammar does: leftover tokens are placed at
the end, unclosed nodes are closed, and misuse (an extra `finish`, a stale
checkpoint) is absorbed rather than panicking. `Debug` shows the next
significant kind, the tokens remaining, and the number of open nodes — handy
when stepping through a grammar. When a single node is being
reparsed, any look past its last token — `peek` returning `None`, `at_end`
returning `true`, a `bump` at the end — marks the attempt as unsafe; test for
closing tokens before testing for the end.

### `Builder::peek`

```rust,ignore
pub fn peek(&self) -> Option<K>
```

The kind of the next significant token, or `None` at the end of input.

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, Char, Missing }
# impl TokenKind for K {}
# struct Chars;
# impl Grammar for Chars {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         for (i, c) in text.char_indices() {
#             tokens.push(Token::new(K::Char, Span::new(i as u32, (i + c.len_utf8()) as u32)));
#         }
#     }
#     fn parse(&self, _: K, b: &mut Builder<'_, K>) {
#         b.start(K::Root);
#         while !b.at_end() { b.bump(); }
#         b.finish();
#     }
# }
// A parse that counts digits by peeking.
struct Digits;
impl Grammar for Digits {
    type Kind = K;
    const ROOT: K = K::Root;
    fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) { Chars.lex(text, tokens) }
    fn parse(&self, _: K, b: &mut Builder<'_, K>) {
        b.start(K::Root);
        while let Some(kind) = b.peek() {
            assert_eq!(kind, K::Char);
            b.bump();
        }
        b.finish();
    }
}
let doc = Document::new(Digits, "123")?;
assert_eq!(doc.tree().root().tokens().count(), 3);
# Ok::<(), incremental_lang::Error>(())
```

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, List, Open, Close, Atom, Space }
# impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
# struct Lists;
# impl Grammar for Lists {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         let bytes = text.as_bytes();
#         let mut i = 0;
#         while i < bytes.len() {
#             let start = i;
#             let kind = match bytes[i] {
#                 b'(' => { i += 1; K::Open }
#                 b')' => { i += 1; K::Close }
#                 b' ' => { while i < bytes.len() && bytes[i] == b' ' { i += 1; } K::Space }
#                 _ => { while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; } K::Atom }
#             };
#             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
#         }
#     }
#     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
#         if kind == K::List { return list(b); }
#         b.start(K::Root);
#         while !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#         b.finish();
#     }
#     fn is_reparsable(&self, kind: K) -> bool { kind == K::List }
# }
# fn list(b: &mut Builder<'_, K>) {
#     b.start(K::List);
#     b.bump();
#     while !b.at(K::Close) && !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#     b.eat(K::Close);
#     b.finish();
# }
// Trivia is skipped: the first thing a parse of "   (a)" sees is `(`.
struct FirstIsOpen;
impl Grammar for FirstIsOpen {
    type Kind = K;
    const ROOT: K = K::Root;
    fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) { Lists.lex(text, tokens) }
    fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
        assert_eq!(b.peek(), Some(K::Open));
        Lists.parse(kind, b);
    }
}
Document::new(FirstIsOpen, "   (a)")?;
# Ok::<(), incremental_lang::Error>(())
```

### `Builder::nth`

```rust,ignore
pub fn nth(&self, n: usize) -> Option<K>
```

The kind of the significant token `n` places ahead; `nth(0)` is
[`peek`](#builderpeek).

| Parameter | Meaning |
|---|---|
| `n` | How many significant tokens to look past. Cost grows with `n` and with the trivia in between. |

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, Char, Missing }
# impl TokenKind for K {}
# struct Chars;
# impl Grammar for Chars {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         for (i, c) in text.char_indices() {
#             tokens.push(Token::new(K::Char, Span::new(i as u32, (i + c.len_utf8()) as u32)));
#         }
#     }
#     fn parse(&self, _: K, b: &mut Builder<'_, K>) {
#         b.start(K::Root);
#         while !b.at_end() { b.bump(); }
#         b.finish();
#     }
# }
struct Look;
impl Grammar for Look {
    type Kind = K;
    const ROOT: K = K::Root;
    fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) { Chars.lex(text, tokens) }
    fn parse(&self, _: K, b: &mut Builder<'_, K>) {
        assert_eq!(b.nth(0), b.peek());
        assert_eq!(b.nth(2), Some(K::Char));
        assert_eq!(b.nth(3), None); // only three tokens
        Chars.parse(K::Root, b);
    }
}
Document::new(Look, "xyz")?;
# Ok::<(), incremental_lang::Error>(())
```

See [`Grammar::parse`](#grammarparse) for `nth(1)` deciding between a pair and
a stray token.

### `Builder::at`

```rust,ignore
pub fn at(&self, kind: K) -> bool
```

Whether the next significant token has kind `kind`. At the end of input it is
`false` (and counts as looking past the end).

| Parameter | Meaning |
|---|---|
| `kind` | The kind to test for, compared with `==`. |

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, List, Open, Close, Atom, Space }
# impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
# struct Lists;
# impl Grammar for Lists {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         let bytes = text.as_bytes();
#         let mut i = 0;
#         while i < bytes.len() {
#             let start = i;
#             let kind = match bytes[i] {
#                 b'(' => { i += 1; K::Open }
#                 b')' => { i += 1; K::Close }
#                 b' ' => { while i < bytes.len() && bytes[i] == b' ' { i += 1; } K::Space }
#                 _ => { while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; } K::Atom }
#             };
#             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
#         }
#     }
#     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
#         if kind == K::List { return list(b); }
#         b.start(K::Root);
#         while !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#         b.finish();
#     }
#     fn is_reparsable(&self, kind: K) -> bool { kind == K::List }
# }
# fn list(b: &mut Builder<'_, K>) {
#     b.start(K::List);
#     b.bump();
#     while !b.at(K::Close) && !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#     b.eat(K::Close);
#     b.finish();
# }
struct Probe;
impl Grammar for Probe {
    type Kind = K;
    const ROOT: K = K::Root;
    fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) { Lists.lex(text, tokens) }
    fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
        assert!(b.at(K::Open));
        assert!(!b.at(K::Atom));
        Lists.parse(kind, b);
    }
}
Document::new(Probe, "(a)")?;
# Ok::<(), incremental_lang::Error>(())
```

### `Builder::at_end`

```rust,ignore
pub fn at_end(&self) -> bool
```

Whether every significant token has been consumed. Write loops over a node's
contents as `while !b.at(Close) && !b.at_end()` — closer first — so that a
well-formed node never looks past its end and stays reparsable on its own.

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, List, Open, Close, Atom, Space }
# impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
# struct Lists;
# impl Grammar for Lists {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         let bytes = text.as_bytes();
#         let mut i = 0;
#         while i < bytes.len() {
#             let start = i;
#             let kind = match bytes[i] {
#                 b'(' => { i += 1; K::Open }
#                 b')' => { i += 1; K::Close }
#                 b' ' => { while i < bytes.len() && bytes[i] == b' ' { i += 1; } K::Space }
#                 _ => { while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; } K::Atom }
#             };
#             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
#         }
#     }
#     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
#         if kind == K::List { return list(b); }
#         b.start(K::Root);
#         while !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#         b.finish();
#     }
#     fn is_reparsable(&self, kind: K) -> bool { kind == K::List }
# }
# fn list(b: &mut Builder<'_, K>) {
#     b.start(K::List);
#     b.bump();
#     while !b.at(K::Close) && !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#     b.eat(K::Close);
#     b.finish();
# }
let mut doc = Document::new(Lists, "(a b) (c)")?;
// `list` tests for `)` before testing for the end, so this stays local.
assert!(!doc.edit(Span::new(3, 4), "z")?.is_full());
# Ok::<(), incremental_lang::Error>(())
```

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, Char, Missing }
# impl TokenKind for K {}
# struct Chars;
# impl Grammar for Chars {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         for (i, c) in text.char_indices() {
#             tokens.push(Token::new(K::Char, Span::new(i as u32, (i + c.len_utf8()) as u32)));
#         }
#     }
#     fn parse(&self, _: K, b: &mut Builder<'_, K>) {
#         b.start(K::Root);
#         while !b.at_end() { b.bump(); }
#         b.finish();
#     }
# }
struct Drain;
impl Grammar for Drain {
    type Kind = K;
    const ROOT: K = K::Root;
    fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) { Chars.lex(text, tokens) }
    fn parse(&self, _: K, b: &mut Builder<'_, K>) {
        b.start(K::Root);
        while !b.at_end() { b.bump(); }
        assert!(b.at_end());
        b.finish();
    }
}
Document::new(Drain, "abc")?;
# Ok::<(), incremental_lang::Error>(())
```

### `Builder::bump`

```rust,ignore
pub fn bump(&mut self)
```

Consumes the next significant token into the node being built, along with any
trivia before it. At the end of input it does nothing.

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, List, Open, Close, Atom, Space }
# impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
# struct Lists;
# impl Grammar for Lists {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         let bytes = text.as_bytes();
#         let mut i = 0;
#         while i < bytes.len() {
#             let start = i;
#             let kind = match bytes[i] {
#                 b'(' => { i += 1; K::Open }
#                 b')' => { i += 1; K::Close }
#                 b' ' => { while i < bytes.len() && bytes[i] == b' ' { i += 1; } K::Space }
#                 _ => { while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; } K::Atom }
#             };
#             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
#         }
#     }
#     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
#         if kind == K::List { return list(b); }
#         b.start(K::Root);
#         while !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#         b.finish();
#     }
#     fn is_reparsable(&self, kind: K) -> bool { kind == K::List }
# }
# fn list(b: &mut Builder<'_, K>) {
#     b.start(K::List);
#     b.bump();
#     while !b.at(K::Close) && !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#     b.eat(K::Close);
#     b.finish();
# }
let doc = Document::new(Lists, "(a  b)")?;
let list = doc.tree().root().descendants().nth(1).ok_or("no list")?;
// The spaces between `a` and `b` were consumed along with `b`.
let kinds: Vec<K> = list.children().map(|c| c.kind()).collect();
assert_eq!(kinds, [K::Open, K::Atom, K::Space, K::Atom, K::Close]);
# Ok::<(), Box<dyn std::error::Error>>(())
```

### `Builder::eat`

```rust,ignore
pub fn eat(&mut self, kind: K) -> bool
```

Consumes the next significant token if it has kind `kind`, and says whether it
did. The natural way to accept an optional token, or to consume a closer that
may be missing.

| Parameter | Meaning |
|---|---|
| `kind` | The kind to accept. |

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, List, Open, Close, Atom, Space }
# impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
# struct Lists;
# impl Grammar for Lists {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         let bytes = text.as_bytes();
#         let mut i = 0;
#         while i < bytes.len() {
#             let start = i;
#             let kind = match bytes[i] {
#                 b'(' => { i += 1; K::Open }
#                 b')' => { i += 1; K::Close }
#                 b' ' => { while i < bytes.len() && bytes[i] == b' ' { i += 1; } K::Space }
#                 _ => { while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; } K::Atom }
#             };
#             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
#         }
#     }
#     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
#         if kind == K::List { return list(b); }
#         b.start(K::Root);
#         while !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#         b.finish();
#     }
#     fn is_reparsable(&self, kind: K) -> bool { kind == K::List }
# }
# fn list(b: &mut Builder<'_, K>) {
#     b.start(K::List);
#     b.bump();
#     while !b.at(K::Close) && !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#     b.eat(K::Close);
#     b.finish();
# }
// `list` ends with `b.eat(K::Close)`: an unclosed list at the end of the
// document still parses, just without its `)`.
let doc = Document::new(Lists, "(a b")?;
let list = doc.tree().root().descendants().nth(1).ok_or("no list")?;
assert_eq!(list.text(doc.text()), Some("(a b"));
# Ok::<(), Box<dyn std::error::Error>>(())
```

### `Builder::start`

```rust,ignore
pub fn start(&mut self, kind: K)
```

Opens a node of kind `kind`; everything consumed until the matching
[`finish`](#builderfinish) becomes its children. Trivia between the previous
token and this node's first token is placed in the enclosing node, ahead of
this one.

| Parameter | Meaning |
|---|---|
| `kind` | The new node's kind. |

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, List, Open, Close, Atom, Space }
# impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
# struct Lists;
# impl Grammar for Lists {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         let bytes = text.as_bytes();
#         let mut i = 0;
#         while i < bytes.len() {
#             let start = i;
#             let kind = match bytes[i] {
#                 b'(' => { i += 1; K::Open }
#                 b')' => { i += 1; K::Close }
#                 b' ' => { while i < bytes.len() && bytes[i] == b' ' { i += 1; } K::Space }
#                 _ => { while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; } K::Atom }
#             };
#             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
#         }
#     }
#     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
#         if kind == K::List { return list(b); }
#         b.start(K::Root);
#         while !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#         b.finish();
#     }
#     fn is_reparsable(&self, kind: K) -> bool { kind == K::List }
# }
# fn list(b: &mut Builder<'_, K>) {
#     b.start(K::List);
#     b.bump();
#     while !b.at(K::Close) && !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#     b.eat(K::Close);
#     b.finish();
# }
let doc = Document::new(Lists, "x   (y)")?;
let list = doc.tree().root().descendants().nth(1).ok_or("no list")?;
// The spaces before `(` stay in the root; the list starts on `(`.
assert_eq!(list.span(), Span::new(4, 7));
# Ok::<(), Box<dyn std::error::Error>>(())
```

An empty node marks something missing and sits right after the last token:

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, Char, Missing }
# impl TokenKind for K {}
# struct Chars;
# impl Grammar for Chars {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         for (i, c) in text.char_indices() {
#             tokens.push(Token::new(K::Char, Span::new(i as u32, (i + c.len_utf8()) as u32)));
#         }
#     }
#     fn parse(&self, _: K, b: &mut Builder<'_, K>) {
#         b.start(K::Root);
#         while !b.at_end() { b.bump(); }
#         b.finish();
#     }
# }
struct Marks;
impl Grammar for Marks {
    type Kind = K;
    const ROOT: K = K::Root;
    fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) { Chars.lex(text, tokens) }
    fn parse(&self, _: K, b: &mut Builder<'_, K>) {
        b.start(K::Root);
        b.bump();
        b.start(K::Missing); // nothing consumed
        b.finish();
        while !b.at_end() { b.bump(); }
        b.finish();
    }
}
let doc = Document::new(Marks, "ab")?;
let missing = doc.tree().root().descendants().find(|n| n.kind() == K::Missing);
assert_eq!(missing.map(|n| n.span()), Some(Span::empty(1)));
# Ok::<(), incremental_lang::Error>(())
```

### `Builder::finish`

```rust,ignore
pub fn finish(&mut self)
```

Closes the most recently opened node. With no node open it does nothing.
Trivia after the node's last token is not taken along; it goes to whichever
node is open when the next token is consumed.

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, List, Open, Close, Atom, Space }
# impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
# struct Lists;
# impl Grammar for Lists {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         let bytes = text.as_bytes();
#         let mut i = 0;
#         while i < bytes.len() {
#             let start = i;
#             let kind = match bytes[i] {
#                 b'(' => { i += 1; K::Open }
#                 b')' => { i += 1; K::Close }
#                 b' ' => { while i < bytes.len() && bytes[i] == b' ' { i += 1; } K::Space }
#                 _ => { while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; } K::Atom }
#             };
#             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
#         }
#     }
#     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
#         if kind == K::List { return list(b); }
#         b.start(K::Root);
#         while !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#         b.finish();
#     }
#     fn is_reparsable(&self, kind: K) -> bool { kind == K::List }
# }
# fn list(b: &mut Builder<'_, K>) {
#     b.start(K::List);
#     b.bump();
#     while !b.at(K::Close) && !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#     b.eat(K::Close);
#     b.finish();
# }
let doc = Document::new(Lists, "(a)   b")?;
let list = doc.tree().root().descendants().nth(1).ok_or("no list")?;
assert_eq!(list.span(), Span::new(0, 3)); // the spaces after `)` are not in it
# Ok::<(), Box<dyn std::error::Error>>(())
```

### `Builder::checkpoint`

```rust,ignore
pub fn checkpoint(&mut self) -> Checkpoint
```

Records the current position among the children of the node being built, for a
later [`start_at`](#builderstart_at). A node opened at the checkpoint never
begins with trivia.

```rust
use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum K { Root, Sum, Num, Plus, Space }
impl TokenKind for K {
    fn is_trivia(&self) -> bool { matches!(self, K::Space) }
}

struct Sums;
impl Grammar for Sums {
    type Kind = K;
    const ROOT: K = K::Root;
    fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
        for (i, c) in text.char_indices() {
            let kind = match c { '+' => K::Plus, ' ' => K::Space, _ => K::Num };
            let end = (i + c.len_utf8()) as u32;
            tokens.push(Token::new(kind, Span::new(i as u32, end)));
        }
    }
    fn parse(&self, _: K, b: &mut Builder<'_, K>) {
        b.start(K::Root);
        let lhs = b.checkpoint();
        b.bump();
        while b.eat(K::Plus) {
            b.bump();
            b.start_at(lhs, K::Sum); // wrap everything since `lhs`
            b.finish();
        }
        b.finish();
    }
}

// Left-associative: ((1 + 2) + 3), and the leading space stays outside.
let doc = Document::new(Sums, " 1 + 2 + 3")?;
let sums: Vec<Span> = doc
    .tree()
    .root()
    .descendants()
    .filter(|n| n.kind() == K::Sum)
    .map(|n| n.span())
    .collect();
assert_eq!(sums, [Span::new(1, 10), Span::new(1, 6)]);
# Ok::<(), incremental_lang::Error>(())
```

### `Builder::start_at`

```rust,ignore
pub fn start_at(&mut self, checkpoint: Checkpoint, kind: K)
```

Opens a node of kind `kind` that begins at `checkpoint`, adopting every child
built since; close it with [`finish`](#builderfinish). This is how a
precedence parser wraps a left operand once it has seen the operator.

| Parameter | Meaning |
|---|---|
| `checkpoint` | From [`checkpoint`](#buildercheckpoint), taken in the node currently being built. One from elsewhere is clamped into range. |
| `kind` | The new node's kind. |

See the [`checkpoint`](#buildercheckpoint) example for an operator chain. A
postfix construct works the same way:

```rust
use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum K { Root, Call, Name, Args }
impl TokenKind for K {}

// `f` is a name; `f()` becomes a Call around the name, decided after it.
struct Calls;
impl Grammar for Calls {
    type Kind = K;
    const ROOT: K = K::Root;
    fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
        let end = text.len() as u32;
        match text.find('(') {
            Some(i) => {
                tokens.push(Token::new(K::Name, Span::new(0, i as u32)));
                tokens.push(Token::new(K::Args, Span::new(i as u32, end)));
            }
            None => tokens.push(Token::new(K::Name, Span::new(0, end))),
        }
    }
    fn parse(&self, _: K, b: &mut Builder<'_, K>) {
        b.start(K::Root);
        let callee = b.checkpoint();
        b.bump();
        if b.at(K::Args) {
            b.start_at(callee, K::Call);
            b.bump();
            b.finish();
        }
        b.finish();
    }
}

let call = Document::new(Calls, "f()")?;
assert!(call.tree().root().descendants().any(|n| n.kind() == K::Call));
let name = Document::new(Calls, "f")?;
assert!(!name.tree().root().descendants().any(|n| n.kind() == K::Call));
# Ok::<(), incremental_lang::Error>(())
```

---

## `Checkpoint`

```rust,ignore
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Checkpoint(/* private */);
```

A position among the children of the node being built, returned by
[`Builder::checkpoint`](#buildercheckpoint) and consumed by
[`Builder::start_at`](#builderstart_at). It is a plain value: keep it in a
local while parsing an operand, and use it once the shape of the construct is
known. Two checkpoints taken with nothing consumed between them are equal.

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, Char, Missing }
# impl TokenKind for K {}
# struct Chars;
# impl Grammar for Chars {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         for (i, c) in text.char_indices() {
#             tokens.push(Token::new(K::Char, Span::new(i as u32, (i + c.len_utf8()) as u32)));
#         }
#     }
#     fn parse(&self, _: K, b: &mut Builder<'_, K>) {
#         b.start(K::Root);
#         while !b.at_end() { b.bump(); }
#         b.finish();
#     }
# }
struct Same;
impl Grammar for Same {
    type Kind = K;
    const ROOT: K = K::Root;
    fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) { Chars.lex(text, tokens) }
    fn parse(&self, _: K, b: &mut Builder<'_, K>) {
        b.start(K::Root);
        let a = b.checkpoint();
        assert_eq!(a, b.checkpoint());
        b.bump();
        assert_ne!(a, b.checkpoint());
        while !b.at_end() { b.bump(); }
        b.finish();
    }
}
Document::new(Same, "xy")?;
# Ok::<(), incremental_lang::Error>(())
```

---

## `Document`

```rust,ignore
pub struct Document<G: Grammar> { /* private */ }
```

A source text and its concrete syntax tree, kept in step under edits. Create
one with [`new`](#documentnew); report every change with
[`edit`](#documentedit); read the result with [`text`](#documenttext) and
[`tree`](#documenttree).

**Cost of an edit.** An edit served by a single node costs a lex and parse of
that node, a walk down the tree to find it, and the splice — in place when no
snapshot shares the path. Updating the text is a `String` splice, a `memmove`
of the text after the edit. Finding the node scans each level's children up to
the edit, so a list that is very wide at one level adds that scan to every
edit beneath it. Failed attempts are budgeted to one full parse's worth of
lexing, so an edit never costs more than about two full parses.

**Snapshots.** [`tree`](#documenttree) borrows the current tree; clone it to
keep a version. While a snapshot is alive, the next edit copies the path it
changes instead of updating it in place, so the snapshot never changes.

`Document<G>` is `Send` and `Sync` when `G` and its kind are. Edits take
`&mut self`; to give other threads something to read while editing continues,
share [snapshots](#documenttree) rather than the document. `Debug` shows the
text length, the root, and the statistics.

### `Document::new`

```rust,ignore
pub fn new(grammar: G, text: impl Into<String>) -> Result<Document<G>, Error>
```

Creates a document, lexing and parsing the whole text once.

| Parameter | Meaning |
|---|---|
| `grammar` | The language. The document owns it; read it back with [`grammar`](#documentgrammar). |
| `text` | The initial text: a `String` (moved, not copied) or anything convertible to one. |

**Errors.** [`Error::TooLarge`](#error) if the text exceeds `u32::MAX` bytes;
[`Error::Tokens`](#error) if the lexer does not tile it.

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, List, Open, Close, Atom, Space }
# impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
# struct Lists;
# impl Grammar for Lists {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         let bytes = text.as_bytes();
#         let mut i = 0;
#         while i < bytes.len() {
#             let start = i;
#             let kind = match bytes[i] {
#                 b'(' => { i += 1; K::Open }
#                 b')' => { i += 1; K::Close }
#                 b' ' => { while i < bytes.len() && bytes[i] == b' ' { i += 1; } K::Space }
#                 _ => { while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; } K::Atom }
#             };
#             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
#         }
#     }
#     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
#         if kind == K::List { return list(b); }
#         b.start(K::Root);
#         while !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#         b.finish();
#     }
#     fn is_reparsable(&self, kind: K) -> bool { kind == K::List }
# }
# fn list(b: &mut Builder<'_, K>) {
#     b.start(K::List);
#     b.bump();
#     while !b.at(K::Close) && !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#     b.eat(K::Close);
#     b.finish();
# }
let doc = Document::new(Lists, String::from("(a b)"))?;
assert_eq!(doc.text(), "(a b)");
assert_eq!(doc.stats().full, 1);
# Ok::<(), incremental_lang::Error>(())
```

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, List, Open, Close, Atom, Space }
# impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
# struct Lists;
# impl Grammar for Lists {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         let bytes = text.as_bytes();
#         let mut i = 0;
#         while i < bytes.len() {
#             let start = i;
#             let kind = match bytes[i] {
#                 b'(' => { i += 1; K::Open }
#                 b')' => { i += 1; K::Close }
#                 b' ' => { while i < bytes.len() && bytes[i] == b' ' { i += 1; } K::Space }
#                 _ => { while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; } K::Atom }
#             };
#             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
#         }
#     }
#     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
#         if kind == K::List { return list(b); }
#         b.start(K::Root);
#         while !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#         b.finish();
#     }
#     fn is_reparsable(&self, kind: K) -> bool { kind == K::List }
# }
# fn list(b: &mut Builder<'_, K>) {
#     b.start(K::List);
#     b.bump();
#     while !b.at(K::Close) && !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#     b.eat(K::Close);
#     b.finish();
# }
// An empty document is a root with no children.
let doc = Document::new(Lists, "")?;
assert_eq!(doc.tree().root().span(), Span::new(0, 0));
assert_eq!(doc.tree().root().children().count(), 0);
# Ok::<(), incremental_lang::Error>(())
```

### `Document::edit`

```rust,ignore
pub fn edit(&mut self, span: Span, text: &str) -> Result<Reparse<G::Kind>, Error>
```

Replaces the bytes in `span` with `text` and brings the tree up to date. The
returned [`Reparse`](#reparse) says what was rebuilt; whatever it was, the tree
afterwards equals a fresh parse of the new text.

| Parameter | Meaning |
|---|---|
| `span` | A byte range of the current text. Empty to insert. Both ends must be on UTF-8 character boundaries. |
| `text` | The replacement. Empty to delete. |

**Errors.** On error the document is unchanged — text, tree, and statistics.
The edit is transactional: text and tree are replaced together once the new
tree is complete, so even a panic in the grammar's own code, caught by the
caller, leaves the document unchanged.
[`Error::OutOfBounds`](#error) if `span` ends past the text;
[`Error::NotCharBoundary`](#error) if an end splits a character;
[`Error::TooLarge`](#error) if the result would exceed `u32::MAX` bytes;
[`Error::Tokens`](#error) if a full reparse was needed and the lexer did not
tile the new text.

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, List, Open, Close, Atom, Space }
# impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
# struct Lists;
# impl Grammar for Lists {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         let bytes = text.as_bytes();
#         let mut i = 0;
#         while i < bytes.len() {
#             let start = i;
#             let kind = match bytes[i] {
#                 b'(' => { i += 1; K::Open }
#                 b')' => { i += 1; K::Close }
#                 b' ' => { while i < bytes.len() && bytes[i] == b' ' { i += 1; } K::Space }
#                 _ => { while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; } K::Atom }
#             };
#             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
#         }
#     }
#     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
#         if kind == K::List { return list(b); }
#         b.start(K::Root);
#         while !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#         b.finish();
#     }
#     fn is_reparsable(&self, kind: K) -> bool { kind == K::List }
# }
# fn list(b: &mut Builder<'_, K>) {
#     b.start(K::List);
#     b.bump();
#     while !b.at(K::Close) && !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#     b.eat(K::Close);
#     b.finish();
# }
let mut doc = Document::new(Lists, "(a b)")?;
doc.edit(Span::new(4, 4), " c")?; // insert
doc.edit(Span::new(1, 2), "z")?;  // replace
doc.edit(Span::new(2, 4), "")?;   // delete
assert_eq!(doc.text(), "(z c)");
assert_eq!(doc.tree(), Document::new(Lists, "(z c)")?.tree());
# Ok::<(), incremental_lang::Error>(())
```

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, List, Open, Close, Atom, Space }
# impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
# struct Lists;
# impl Grammar for Lists {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         let bytes = text.as_bytes();
#         let mut i = 0;
#         while i < bytes.len() {
#             let start = i;
#             let kind = match bytes[i] {
#                 b'(' => { i += 1; K::Open }
#                 b')' => { i += 1; K::Close }
#                 b' ' => { while i < bytes.len() && bytes[i] == b' ' { i += 1; } K::Space }
#                 _ => { while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; } K::Atom }
#             };
#             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
#         }
#     }
#     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
#         if kind == K::List { return list(b); }
#         b.start(K::Root);
#         while !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#         b.finish();
#     }
#     fn is_reparsable(&self, kind: K) -> bool { kind == K::List }
# }
# fn list(b: &mut Builder<'_, K>) {
#     b.start(K::List);
#     b.bump();
#     while !b.at(K::Close) && !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#     b.eat(K::Close);
#     b.finish();
# }
use incremental_lang::Error;

let mut doc = Document::new(Lists, "(é)")?;
// `é` occupies bytes 1..3.
assert_eq!(doc.edit(Span::new(2, 2), "x"), Err(Error::NotCharBoundary { offset: 2 }));
assert_eq!(
    doc.edit(Span::new(0, 99), ""),
    Err(Error::OutOfBounds { span: Span::new(0, 99), len: 4 }),
);
assert_eq!(doc.text(), "(é)");
assert_eq!(doc.stats().edits, 0);
# Ok::<(), Error>(())
```

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, List, Open, Close, Atom, Space }
# impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
# struct Lists;
# impl Grammar for Lists {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         let bytes = text.as_bytes();
#         let mut i = 0;
#         while i < bytes.len() {
#             let start = i;
#             let kind = match bytes[i] {
#                 b'(' => { i += 1; K::Open }
#                 b')' => { i += 1; K::Close }
#                 b' ' => { while i < bytes.len() && bytes[i] == b' ' { i += 1; } K::Space }
#                 _ => { while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; } K::Atom }
#             };
#             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
#         }
#     }
#     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
#         if kind == K::List { return list(b); }
#         b.start(K::Root);
#         while !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#         b.finish();
#     }
#     fn is_reparsable(&self, kind: K) -> bool { kind == K::List }
# }
# fn list(b: &mut Builder<'_, K>) {
#     b.start(K::List);
#     b.bump();
#     while !b.at(K::Close) && !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#     b.eat(K::Close);
#     b.finish();
# }
// Replacing the whole text is one edit — the resync an editor uses when it
// has lost track of incremental changes.
let mut doc = Document::new(Lists, "(a)")?;
let len = doc.text().len() as u32;
let edit = doc.edit(Span::new(0, len), "(b (c))")?;
assert!(edit.is_full());
assert_eq!(doc.text(), "(b (c))");
# Ok::<(), incremental_lang::Error>(())
```

### `Document::text`

```rust,ignore
pub fn text(&self) -> &str
```

The current text, as one contiguous string — pass it to
[`Node::text`](#nodetext) to read a node's source.

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, List, Open, Close, Atom, Space }
# impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
# struct Lists;
# impl Grammar for Lists {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         let bytes = text.as_bytes();
#         let mut i = 0;
#         while i < bytes.len() {
#             let start = i;
#             let kind = match bytes[i] {
#                 b'(' => { i += 1; K::Open }
#                 b')' => { i += 1; K::Close }
#                 b' ' => { while i < bytes.len() && bytes[i] == b' ' { i += 1; } K::Space }
#                 _ => { while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; } K::Atom }
#             };
#             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
#         }
#     }
#     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
#         if kind == K::List { return list(b); }
#         b.start(K::Root);
#         while !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#         b.finish();
#     }
#     fn is_reparsable(&self, kind: K) -> bool { kind == K::List }
# }
# fn list(b: &mut Builder<'_, K>) {
#     b.start(K::List);
#     b.bump();
#     while !b.at(K::Close) && !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#     b.eat(K::Close);
#     b.finish();
# }
let mut doc = Document::new(Lists, "(a)")?;
doc.edit(Span::new(2, 2), " b")?;
assert_eq!(doc.text(), "(a b)");
# Ok::<(), incremental_lang::Error>(())
```

### `Document::tree`

```rust,ignore
pub fn tree(&self) -> &Tree<G::Kind>
```

The current syntax tree. Borrow it to read; clone it to keep a snapshot.

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, List, Open, Close, Atom, Space }
# impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
# struct Lists;
# impl Grammar for Lists {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         let bytes = text.as_bytes();
#         let mut i = 0;
#         while i < bytes.len() {
#             let start = i;
#             let kind = match bytes[i] {
#                 b'(' => { i += 1; K::Open }
#                 b')' => { i += 1; K::Close }
#                 b' ' => { while i < bytes.len() && bytes[i] == b' ' { i += 1; } K::Space }
#                 _ => { while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; } K::Atom }
#             };
#             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
#         }
#     }
#     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
#         if kind == K::List { return list(b); }
#         b.start(K::Root);
#         while !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#         b.finish();
#     }
#     fn is_reparsable(&self, kind: K) -> bool { kind == K::List }
# }
# fn list(b: &mut Builder<'_, K>) {
#     b.start(K::List);
#     b.bump();
#     while !b.at(K::Close) && !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#     b.eat(K::Close);
#     b.finish();
# }
let mut doc = Document::new(Lists, "(a)")?;
let snapshot = doc.tree().clone(); // O(1)
doc.edit(Span::new(1, 2), "a b")?;
assert_eq!(snapshot.root().span(), Span::new(0, 3));
assert_eq!(doc.tree().root().span(), Span::new(0, 5));
# Ok::<(), incremental_lang::Error>(())
```

### `Document::grammar`

```rust,ignore
pub fn grammar(&self) -> &G
```

The grammar the document parses with — useful when the grammar carries
configuration.

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, Char, Missing }
# impl TokenKind for K {}
# struct Chars;
# impl Grammar for Chars {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         for (i, c) in text.char_indices() {
#             tokens.push(Token::new(K::Char, Span::new(i as u32, (i + c.len_utf8()) as u32)));
#         }
#     }
#     fn parse(&self, _: K, b: &mut Builder<'_, K>) {
#         b.start(K::Root);
#         while !b.at_end() { b.bump(); }
#         b.finish();
#     }
# }
let doc = Document::new(Chars, "x")?;
let _grammar: &Chars = doc.grammar();
assert!(!doc.grammar().is_reparsable(K::Root));
# Ok::<(), incremental_lang::Error>(())
```

### `Document::stats`

```rust,ignore
pub fn stats(&self) -> Stats
```

A copy of the document's running counters; see [`Stats`](#stats).

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, List, Open, Close, Atom, Space }
# impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
# struct Lists;
# impl Grammar for Lists {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         let bytes = text.as_bytes();
#         let mut i = 0;
#         while i < bytes.len() {
#             let start = i;
#             let kind = match bytes[i] {
#                 b'(' => { i += 1; K::Open }
#                 b')' => { i += 1; K::Close }
#                 b' ' => { while i < bytes.len() && bytes[i] == b' ' { i += 1; } K::Space }
#                 _ => { while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; } K::Atom }
#             };
#             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
#         }
#     }
#     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
#         if kind == K::List { return list(b); }
#         b.start(K::Root);
#         while !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#         b.finish();
#     }
#     fn is_reparsable(&self, kind: K) -> bool { kind == K::List }
# }
# fn list(b: &mut Builder<'_, K>) {
#     b.start(K::List);
#     b.bump();
#     while !b.at(K::Close) && !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#     b.eat(K::Close);
#     b.finish();
# }
let mut doc = Document::new(Lists, "(a b) (c d)")?;
doc.edit(Span::new(3, 4), "x")?;
let stats = doc.stats();
assert_eq!((stats.edits, stats.partial, stats.full), (1, 1, 1));
# Ok::<(), incremental_lang::Error>(())
```

---

## `Reparse`

```rust,ignore
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reparse<K> { /* private */ }

impl<K: Copy> Reparse<K> {
    pub const fn kind(&self) -> K;
    pub const fn span(&self) -> Span;
    pub const fn is_full(&self) -> bool;
}
```

What one call to [`Document::edit`](#documentedit) rebuilt.

| Method | Returns |
|---|---|
| `kind()` | The kind of the rebuilt node: a reparsable kind, or [`ROOT`](#grammarroot) after a full reparse. |
| `span()` | The region of the *new* text whose syntax was rebuilt. Everything outside it kept its tree, shifted by the edit. |
| `is_full()` | Whether the whole document was reparsed. |

An editor can confine re-highlighting, re-folding, or re-checking to `span()`.

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, List, Open, Close, Atom, Space }
# impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
# struct Lists;
# impl Grammar for Lists {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         let bytes = text.as_bytes();
#         let mut i = 0;
#         while i < bytes.len() {
#             let start = i;
#             let kind = match bytes[i] {
#                 b'(' => { i += 1; K::Open }
#                 b')' => { i += 1; K::Close }
#                 b' ' => { while i < bytes.len() && bytes[i] == b' ' { i += 1; } K::Space }
#                 _ => { while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; } K::Atom }
#             };
#             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
#         }
#     }
#     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
#         if kind == K::List { return list(b); }
#         b.start(K::Root);
#         while !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#         b.finish();
#     }
#     fn is_reparsable(&self, kind: K) -> bool { kind == K::List }
# }
# fn list(b: &mut Builder<'_, K>) {
#     b.start(K::List);
#     b.bump();
#     while !b.at(K::Close) && !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#     b.eat(K::Close);
#     b.finish();
# }
let mut doc = Document::new(Lists, "(a) (b c)")?;

let local = doc.edit(Span::new(6, 6), "x")?;
assert_eq!((local.kind(), local.span(), local.is_full()), (K::List, Span::new(4, 10), false));

let global = doc.edit(Span::new(0, 1), "")?; // unbalances the text
assert_eq!((global.kind(), global.span(), global.is_full()), (K::Root, Span::new(0, 9), true));
# Ok::<(), incremental_lang::Error>(())
```

---

## `Stats`

```rust,ignore
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct Stats {
    pub edits: u64,
    pub partial: u64,
    pub full: u64,
    pub rejected: u64,
    pub bytes: u64,
}
```

Running counters a document keeps about its own work. All start at zero and
only grow.

| Field | Counts | What to watch |
|---|---|---|
| `edits` | Edits applied. Refused edits are not counted. | — |
| `partial` | Edits served by reparsing a single node. | Should dominate during ordinary typing. |
| `full` | Whole-document parses, including the one in [`new`](#documentnew). | Rises with edits that unbalance or re-lex large regions. |
| `rejected` | Single-node attempts refused by a safety check. | Steady growth means the grammar's reparsable kinds rarely survive real edits. |
| `bytes` | Bytes handed to the lexer across all parses and attempts. | Divided by `edits`, the average work per edit. |

`edits == partial + full - 1` always holds. The struct is `#[non_exhaustive]`:
read its fields, but do not construct it.

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, List, Open, Close, Atom, Space }
# impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
# struct Lists;
# impl Grammar for Lists {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         let bytes = text.as_bytes();
#         let mut i = 0;
#         while i < bytes.len() {
#             let start = i;
#             let kind = match bytes[i] {
#                 b'(' => { i += 1; K::Open }
#                 b')' => { i += 1; K::Close }
#                 b' ' => { while i < bytes.len() && bytes[i] == b' ' { i += 1; } K::Space }
#                 _ => { while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; } K::Atom }
#             };
#             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
#         }
#     }
#     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
#         if kind == K::List { return list(b); }
#         b.start(K::Root);
#         while !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#         b.finish();
#     }
#     fn is_reparsable(&self, kind: K) -> bool { kind == K::List }
# }
# fn list(b: &mut Builder<'_, K>) {
#     b.start(K::List);
#     b.bump();
#     while !b.at(K::Close) && !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#     b.eat(K::Close);
#     b.finish();
# }
let mut doc = Document::new(Lists, "(a b) (c d)")?;
doc.edit(Span::new(2, 3), "x")?;   // inside the first list
doc.edit(Span::new(8, 9), "y")?;   // inside the second
doc.edit(Span::new(0, 1), "")?;    // unbalanced: full reparse

let stats = doc.stats();
assert_eq!(stats.edits, 3);
assert_eq!(stats.partial, 2);
assert_eq!(stats.full, 2); // the initial parse and the last edit
assert_eq!(stats.edits, stats.partial + stats.full - 1);
# Ok::<(), incremental_lang::Error>(())
```

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, List, Open, Close, Atom, Space }
# impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
# struct Lists;
# impl Grammar for Lists {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         let bytes = text.as_bytes();
#         let mut i = 0;
#         while i < bytes.len() {
#             let start = i;
#             let kind = match bytes[i] {
#                 b'(' => { i += 1; K::Open }
#                 b')' => { i += 1; K::Close }
#                 b' ' => { while i < bytes.len() && bytes[i] == b' ' { i += 1; } K::Space }
#                 _ => { while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; } K::Atom }
#             };
#             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
#         }
#     }
#     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
#         if kind == K::List { return list(b); }
#         b.start(K::Root);
#         while !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#         b.finish();
#     }
#     fn is_reparsable(&self, kind: K) -> bool { kind == K::List }
# }
# fn list(b: &mut Builder<'_, K>) {
#     b.start(K::List);
#     b.bump();
#     while !b.at(K::Close) && !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#     b.eat(K::Close);
#     b.finish();
# }
// Bytes lexed per edit, a direct measure of incrementality.
let mut doc = Document::new(Lists, "(a) (b) (c) (d) (e) (f) (g)")?;
let before = doc.stats().bytes;
doc.edit(Span::new(17, 18), "z")?; // inside `(e)`
let lexed = doc.stats().bytes - before;
assert!(lexed < 8, "relexed {lexed} bytes"); // `(z)` and its neighbours
# Ok::<(), incremental_lang::Error>(())
```

---

## `Tree`

```rust,ignore
#[derive(Clone)]
pub struct Tree<K> { /* private */ }

impl<K: PartialEq> PartialEq for Tree<K>;
impl<K: Eq> Eq for Tree<K>;
impl<K: Copy + Debug> Debug for Tree<K>;
```

An immutable snapshot of a document's concrete syntax tree. Cloning is one
reference-count increment; the clone never changes; and `Tree<K>` is
`Send + Sync` when `K` is, so it can be handed to another thread. Successive
versions share every subtree the edits between them did not touch. A tree
holds no text: positions come from [`root`](#treeroot), text from the source
passed to [`Node::text`](#nodetext).

**Equality** is structural — same kinds, same spans, same shape. It does not
compare text: renaming `a` to `b` yields an equal tree. It is how the tests
check incremental results against fresh parses, and it skips subtrees that two
trees share.

**Debug** prints an indented outline, one node or token per line, for people
(not a stable format):

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, List, Open, Close, Atom, Space }
# impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
# struct Lists;
# impl Grammar for Lists {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         let bytes = text.as_bytes();
#         let mut i = 0;
#         while i < bytes.len() {
#             let start = i;
#             let kind = match bytes[i] {
#                 b'(' => { i += 1; K::Open }
#                 b')' => { i += 1; K::Close }
#                 b' ' => { while i < bytes.len() && bytes[i] == b' ' { i += 1; } K::Space }
#                 _ => { while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; } K::Atom }
#             };
#             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
#         }
#     }
#     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
#         if kind == K::List { return list(b); }
#         b.start(K::Root);
#         while !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#         b.finish();
#     }
#     fn is_reparsable(&self, kind: K) -> bool { kind == K::List }
# }
# fn list(b: &mut Builder<'_, K>) {
#     b.start(K::List);
#     b.bump();
#     while !b.at(K::Close) && !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#     b.eat(K::Close);
#     b.finish();
# }
let doc = Document::new(Lists, "(a b)")?;
let outline = format!("{:?}", doc.tree());
assert_eq!(
    outline,
    "Root@0..5\n  List@0..5\n    Open@0..1\n    Atom@1..2\n    Space@2..3\n    Atom@3..4\n    Close@4..5\n",
);
# Ok::<(), incremental_lang::Error>(())
```

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, List, Open, Close, Atom, Space }
# impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
# struct Lists;
# impl Grammar for Lists {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         let bytes = text.as_bytes();
#         let mut i = 0;
#         while i < bytes.len() {
#             let start = i;
#             let kind = match bytes[i] {
#                 b'(' => { i += 1; K::Open }
#                 b')' => { i += 1; K::Close }
#                 b' ' => { while i < bytes.len() && bytes[i] == b' ' { i += 1; } K::Space }
#                 _ => { while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; } K::Atom }
#             };
#             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
#         }
#     }
#     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
#         if kind == K::List { return list(b); }
#         b.start(K::Root);
#         while !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#         b.finish();
#     }
#     fn is_reparsable(&self, kind: K) -> bool { kind == K::List }
# }
# fn list(b: &mut Builder<'_, K>) {
#     b.start(K::List);
#     b.bump();
#     while !b.at(K::Close) && !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#     b.eat(K::Close);
#     b.finish();
# }
let mut doc = Document::new(Lists, "(a) (b)")?;
let before = doc.tree().clone();
doc.edit(Span::new(5, 6), "b c")?;
assert_ne!(&before, doc.tree());
doc.edit(Span::new(5, 8), "b")?; // undo
assert_eq!(&before, doc.tree());
# Ok::<(), incremental_lang::Error>(())
```

### `Tree::root`

```rust,ignore
pub fn root(&self) -> Node<'_, K>
```

The root node, spanning the whole document at offset 0. Every read starts here.

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, List, Open, Close, Atom, Space }
# impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
# struct Lists;
# impl Grammar for Lists {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         let bytes = text.as_bytes();
#         let mut i = 0;
#         while i < bytes.len() {
#             let start = i;
#             let kind = match bytes[i] {
#                 b'(' => { i += 1; K::Open }
#                 b')' => { i += 1; K::Close }
#                 b' ' => { while i < bytes.len() && bytes[i] == b' ' { i += 1; } K::Space }
#                 _ => { while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; } K::Atom }
#             };
#             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
#         }
#     }
#     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
#         if kind == K::List { return list(b); }
#         b.start(K::Root);
#         while !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#         b.finish();
#     }
#     fn is_reparsable(&self, kind: K) -> bool { kind == K::List }
# }
# fn list(b: &mut Builder<'_, K>) {
#     b.start(K::List);
#     b.bump();
#     while !b.at(K::Close) && !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#     b.eat(K::Close);
#     b.finish();
# }
let doc = Document::new(Lists, "(a) b")?;
let root = doc.tree().root();
assert_eq!(root.kind(), K::Root);
assert_eq!(root.span(), Span::new(0, 5));
assert_eq!(root.text(doc.text()), Some("(a) b"));
# Ok::<(), incremental_lang::Error>(())
```

### `Tree::to_syntax`

```rust,ignore
pub fn to_syntax(&self) -> syntax_lang::Node<K>
```

Converts the tree into a [`syntax_lang::Node`](https://docs.rs/syntax-lang) —
the `-lang` family's standard CST, with absolute spans — for tooling built on
`syntax-lang`. The conversion copies the whole tree, so it suits handing off a
finished version, not reading after every keystroke. It is iterative and safe
on any depth.

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, List, Open, Close, Atom, Space }
# impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
# struct Lists;
# impl Grammar for Lists {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         let bytes = text.as_bytes();
#         let mut i = 0;
#         while i < bytes.len() {
#             let start = i;
#             let kind = match bytes[i] {
#                 b'(' => { i += 1; K::Open }
#                 b')' => { i += 1; K::Close }
#                 b' ' => { while i < bytes.len() && bytes[i] == b' ' { i += 1; } K::Space }
#                 _ => { while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; } K::Atom }
#             };
#             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
#         }
#     }
#     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
#         if kind == K::List { return list(b); }
#         b.start(K::Root);
#         while !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#         b.finish();
#     }
#     fn is_reparsable(&self, kind: K) -> bool { kind == K::List }
# }
# fn list(b: &mut Builder<'_, K>) {
#     b.start(K::List);
#     b.bump();
#     while !b.at(K::Close) && !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#     b.eat(K::Close);
#     b.finish();
# }
let doc = Document::new(Lists, "(a (b))")?;
let cst = doc.tree().to_syntax();
assert_eq!(cst.kind(), &K::Root);
assert_eq!(cst.text(doc.text()), Some("(a (b))"));
assert_eq!(cst.descendants().filter(|n| *n.kind() == K::List).count(), 2);
# Ok::<(), incremental_lang::Error>(())
```

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, List, Open, Close, Atom, Space }
# impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
# struct Lists;
# impl Grammar for Lists {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         let bytes = text.as_bytes();
#         let mut i = 0;
#         while i < bytes.len() {
#             let start = i;
#             let kind = match bytes[i] {
#                 b'(' => { i += 1; K::Open }
#                 b')' => { i += 1; K::Close }
#                 b' ' => { while i < bytes.len() && bytes[i] == b' ' { i += 1; } K::Space }
#                 _ => { while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; } K::Atom }
#             };
#             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
#         }
#     }
#     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
#         if kind == K::List { return list(b); }
#         b.start(K::Root);
#         while !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#         b.finish();
#     }
#     fn is_reparsable(&self, kind: K) -> bool { kind == K::List }
# }
# fn list(b: &mut Builder<'_, K>) {
#     b.start(K::List);
#     b.bump();
#     while !b.at(K::Close) && !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#     b.eat(K::Close);
#     b.finish();
# }
// Same tokens, same spans, in the same order.
let doc = Document::new(Lists, "(x y) z")?;
let ours: Vec<_> = doc.tree().root().tokens().collect();
let theirs: Vec<_> = doc.tree().to_syntax().tokens().copied().collect();
assert_eq!(ours, theirs);
# Ok::<(), incremental_lang::Error>(())
```

---

## `Node`

```rust,ignore
pub struct Node<'a, K> { /* private */ }   // Copy
```

A node of a [`Tree`](#tree), positioned in the document: a reference into the
tree and the node's absolute start. It is `Copy`, its methods take `self` by
value, and the iterators they return borrow only the tree. `Debug` prints
`Kind@start..end`.

### `Node::kind`

```rust,ignore
pub fn kind(self) -> K
```

The node's kind.

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, List, Open, Close, Atom, Space }
# impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
# struct Lists;
# impl Grammar for Lists {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         let bytes = text.as_bytes();
#         let mut i = 0;
#         while i < bytes.len() {
#             let start = i;
#             let kind = match bytes[i] {
#                 b'(' => { i += 1; K::Open }
#                 b')' => { i += 1; K::Close }
#                 b' ' => { while i < bytes.len() && bytes[i] == b' ' { i += 1; } K::Space }
#                 _ => { while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; } K::Atom }
#             };
#             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
#         }
#     }
#     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
#         if kind == K::List { return list(b); }
#         b.start(K::Root);
#         while !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#         b.finish();
#     }
#     fn is_reparsable(&self, kind: K) -> bool { kind == K::List }
# }
# fn list(b: &mut Builder<'_, K>) {
#     b.start(K::List);
#     b.bump();
#     while !b.at(K::Close) && !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#     b.eat(K::Close);
#     b.finish();
# }
let doc = Document::new(Lists, "(a)")?;
let kinds: Vec<K> = doc.tree().root().descendants().map(|n| n.kind()).collect();
assert_eq!(kinds, [K::Root, K::List]);
# Ok::<(), incremental_lang::Error>(())
```

### `Node::span`

```rust,ignore
pub fn span(self) -> Span
```

The bytes the node covers. Except for the root, a node begins on its first
significant token and ends on its last; a node with no tokens has an empty span
where it sits.

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, List, Open, Close, Atom, Space }
# impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
# struct Lists;
# impl Grammar for Lists {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         let bytes = text.as_bytes();
#         let mut i = 0;
#         while i < bytes.len() {
#             let start = i;
#             let kind = match bytes[i] {
#                 b'(' => { i += 1; K::Open }
#                 b')' => { i += 1; K::Close }
#                 b' ' => { while i < bytes.len() && bytes[i] == b' ' { i += 1; } K::Space }
#                 _ => { while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; } K::Atom }
#             };
#             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
#         }
#     }
#     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
#         if kind == K::List { return list(b); }
#         b.start(K::Root);
#         while !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#         b.finish();
#     }
#     fn is_reparsable(&self, kind: K) -> bool { kind == K::List }
# }
# fn list(b: &mut Builder<'_, K>) {
#     b.start(K::List);
#     b.bump();
#     while !b.at(K::Close) && !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#     b.eat(K::Close);
#     b.finish();
# }
let doc = Document::new(Lists, "  (a b)  ")?;
let list = doc.tree().root().descendants().nth(1).ok_or("no list")?;
assert_eq!(list.span(), Span::new(2, 7));
assert_eq!(doc.tree().root().span(), Span::new(0, 9));
# Ok::<(), Box<dyn std::error::Error>>(())
```

### `Node::text`

```rust,ignore
pub fn text(self, source: &str) -> Option<&str>
```

The node's text, borrowed from `source` — zero-copy.

| Parameter | Meaning |
|---|---|
| `source` | The text the tree describes: [`Document::text`](#documenttext), or the copy kept alongside a snapshot. |

Returns `None` if the span is not on character boundaries inside `source`,
which means `source` is not the tree's text.

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, List, Open, Close, Atom, Space }
# impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
# struct Lists;
# impl Grammar for Lists {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         let bytes = text.as_bytes();
#         let mut i = 0;
#         while i < bytes.len() {
#             let start = i;
#             let kind = match bytes[i] {
#                 b'(' => { i += 1; K::Open }
#                 b')' => { i += 1; K::Close }
#                 b' ' => { while i < bytes.len() && bytes[i] == b' ' { i += 1; } K::Space }
#                 _ => { while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; } K::Atom }
#             };
#             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
#         }
#     }
#     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
#         if kind == K::List { return list(b); }
#         b.start(K::Root);
#         while !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#         b.finish();
#     }
#     fn is_reparsable(&self, kind: K) -> bool { kind == K::List }
# }
# fn list(b: &mut Builder<'_, K>) {
#     b.start(K::List);
#     b.bump();
#     while !b.at(K::Close) && !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#     b.eat(K::Close);
#     b.finish();
# }
let doc = Document::new(Lists, "(a) (b c)")?;
let texts: Vec<&str> = doc
    .tree()
    .root()
    .descendants()
    .skip(1)
    .filter_map(|n| n.text(doc.text()))
    .collect();
assert_eq!(texts, ["(a)", "(b c)"]);
assert_eq!(doc.tree().root().text("short"), None);
# Ok::<(), incremental_lang::Error>(())
```

### `Node::children`

```rust,ignore
pub fn children(self) -> impl ExactSizeIterator<Item = Element<'a, K>>
```

The node's direct children — nodes and tokens in source order, with absolute
spans.

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, List, Open, Close, Atom, Space }
# impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
# struct Lists;
# impl Grammar for Lists {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         let bytes = text.as_bytes();
#         let mut i = 0;
#         while i < bytes.len() {
#             let start = i;
#             let kind = match bytes[i] {
#                 b'(' => { i += 1; K::Open }
#                 b')' => { i += 1; K::Close }
#                 b' ' => { while i < bytes.len() && bytes[i] == b' ' { i += 1; } K::Space }
#                 _ => { while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; } K::Atom }
#             };
#             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
#         }
#     }
#     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
#         if kind == K::List { return list(b); }
#         b.start(K::Root);
#         while !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#         b.finish();
#     }
#     fn is_reparsable(&self, kind: K) -> bool { kind == K::List }
# }
# fn list(b: &mut Builder<'_, K>) {
#     b.start(K::List);
#     b.bump();
#     while !b.at(K::Close) && !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#     b.eat(K::Close);
#     b.finish();
# }
let doc = Document::new(Lists, "(a) b")?;
let children: Vec<(K, Span)> = doc
    .tree()
    .root()
    .children()
    .map(|c| (c.kind(), c.span()))
    .collect();
assert_eq!(
    children,
    [(K::List, Span::new(0, 3)), (K::Space, Span::new(3, 4)), (K::Atom, Span::new(4, 5))],
);
assert_eq!(doc.tree().root().children().len(), 3);
# Ok::<(), incremental_lang::Error>(())
```

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, List, Open, Close, Atom, Space }
# impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
# struct Lists;
# impl Grammar for Lists {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         let bytes = text.as_bytes();
#         let mut i = 0;
#         while i < bytes.len() {
#             let start = i;
#             let kind = match bytes[i] {
#                 b'(' => { i += 1; K::Open }
#                 b')' => { i += 1; K::Close }
#                 b' ' => { while i < bytes.len() && bytes[i] == b' ' { i += 1; } K::Space }
#                 _ => { while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; } K::Atom }
#             };
#             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
#         }
#     }
#     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
#         if kind == K::List { return list(b); }
#         b.start(K::Root);
#         while !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#         b.finish();
#     }
#     fn is_reparsable(&self, kind: K) -> bool { kind == K::List }
# }
# fn list(b: &mut Builder<'_, K>) {
#     b.start(K::List);
#     b.bump();
#     while !b.at(K::Close) && !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#     b.eat(K::Close);
#     b.finish();
# }
use incremental_lang::Element;

// Only the nested nodes.
let doc = Document::new(Lists, "((a) (b))")?;
let outer = doc.tree().root().children().find_map(Element::as_node).ok_or("no list")?;
let inner: Vec<Span> = outer.children().filter_map(Element::as_node).map(|n| n.span()).collect();
assert_eq!(inner, [Span::new(1, 4), Span::new(5, 8)]);
# Ok::<(), Box<dyn std::error::Error>>(())
```

### `Node::descendants`

```rust,ignore
pub fn descendants(self) -> impl Iterator<Item = Node<'a, K>>
```

This node and every node beneath it, in pre-order. Iterative: safe on any
depth.

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, List, Open, Close, Atom, Space }
# impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
# struct Lists;
# impl Grammar for Lists {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         let bytes = text.as_bytes();
#         let mut i = 0;
#         while i < bytes.len() {
#             let start = i;
#             let kind = match bytes[i] {
#                 b'(' => { i += 1; K::Open }
#                 b')' => { i += 1; K::Close }
#                 b' ' => { while i < bytes.len() && bytes[i] == b' ' { i += 1; } K::Space }
#                 _ => { while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; } K::Atom }
#             };
#             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
#         }
#     }
#     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
#         if kind == K::List { return list(b); }
#         b.start(K::Root);
#         while !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#         b.finish();
#     }
#     fn is_reparsable(&self, kind: K) -> bool { kind == K::List }
# }
# fn list(b: &mut Builder<'_, K>) {
#     b.start(K::List);
#     b.bump();
#     while !b.at(K::Close) && !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#     b.eat(K::Close);
#     b.finish();
# }
let doc = Document::new(Lists, "(a (b (c)))")?;
let depths: Vec<Span> = doc
    .tree()
    .root()
    .descendants()
    .filter(|n| n.kind() == K::List)
    .map(|n| n.span())
    .collect();
assert_eq!(depths, [Span::new(0, 11), Span::new(3, 10), Span::new(6, 9)]);
# Ok::<(), incremental_lang::Error>(())
```

### `Node::tokens`

```rust,ignore
pub fn tokens(self) -> impl Iterator<Item = Token<K>>
```

Every token beneath the node in source order, trivia included, with absolute
spans. Concatenating their text reproduces the node's text. Iterative.

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, List, Open, Close, Atom, Space }
# impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
# struct Lists;
# impl Grammar for Lists {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         let bytes = text.as_bytes();
#         let mut i = 0;
#         while i < bytes.len() {
#             let start = i;
#             let kind = match bytes[i] {
#                 b'(' => { i += 1; K::Open }
#                 b')' => { i += 1; K::Close }
#                 b' ' => { while i < bytes.len() && bytes[i] == b' ' { i += 1; } K::Space }
#                 _ => { while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; } K::Atom }
#             };
#             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
#         }
#     }
#     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
#         if kind == K::List { return list(b); }
#         b.start(K::Root);
#         while !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#         b.finish();
#     }
#     fn is_reparsable(&self, kind: K) -> bool { kind == K::List }
# }
# fn list(b: &mut Builder<'_, K>) {
#     b.start(K::List);
#     b.bump();
#     while !b.at(K::Close) && !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#     b.eat(K::Close);
#     b.finish();
# }
let doc = Document::new(Lists, "(a  b)")?;
let rebuilt: String = doc
    .tree()
    .root()
    .tokens()
    .map(|t| &doc.text()[t.span.start().to_usize()..t.span.end().to_usize()])
    .collect();
assert_eq!(rebuilt, doc.text());
# Ok::<(), incremental_lang::Error>(())
```

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, List, Open, Close, Atom, Space }
# impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
# struct Lists;
# impl Grammar for Lists {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         let bytes = text.as_bytes();
#         let mut i = 0;
#         while i < bytes.len() {
#             let start = i;
#             let kind = match bytes[i] {
#                 b'(' => { i += 1; K::Open }
#                 b')' => { i += 1; K::Close }
#                 b' ' => { while i < bytes.len() && bytes[i] == b' ' { i += 1; } K::Space }
#                 _ => { while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; } K::Atom }
#             };
#             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
#         }
#     }
#     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
#         if kind == K::List { return list(b); }
#         b.start(K::Root);
#         while !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#         b.finish();
#     }
#     fn is_reparsable(&self, kind: K) -> bool { kind == K::List }
# }
# fn list(b: &mut Builder<'_, K>) {
#     b.start(K::List);
#     b.bump();
#     while !b.at(K::Close) && !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#     b.eat(K::Close);
#     b.finish();
# }
// Significant tokens only.
let doc = Document::new(Lists, "( a  b )")?;
let significant = doc.tree().root().tokens().filter(|t| !t.is_trivia()).count();
assert_eq!(significant, 4);
# Ok::<(), incremental_lang::Error>(())
```

---

## `Element`

```rust,ignore
pub enum Element<'a, K> {
    Node(Node<'a, K>),
    Token(Token<K>),
}

impl<'a, K: Copy> Element<'a, K> {
    pub fn kind(self) -> K;
    pub fn span(self) -> Span;
    pub fn as_node(self) -> Option<Node<'a, K>>;
    pub fn as_token(self) -> Option<Token<K>>;
}
```

One child of a [`Node`](#node): a nested node, or a leaf token with its
absolute span. `Copy`; `Debug` prints `Kind@start..end`.

| Method | Returns |
|---|---|
| `kind()` | The child's kind, node or token alike. |
| `span()` | The bytes it covers. |
| `as_node()` | The node, if it is one. |
| `as_token()` | The token, if it is one. |

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, List, Open, Close, Atom, Space }
# impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
# struct Lists;
# impl Grammar for Lists {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         let bytes = text.as_bytes();
#         let mut i = 0;
#         while i < bytes.len() {
#             let start = i;
#             let kind = match bytes[i] {
#                 b'(' => { i += 1; K::Open }
#                 b')' => { i += 1; K::Close }
#                 b' ' => { while i < bytes.len() && bytes[i] == b' ' { i += 1; } K::Space }
#                 _ => { while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; } K::Atom }
#             };
#             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
#         }
#     }
#     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
#         if kind == K::List { return list(b); }
#         b.start(K::Root);
#         while !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#         b.finish();
#     }
#     fn is_reparsable(&self, kind: K) -> bool { kind == K::List }
# }
# fn list(b: &mut Builder<'_, K>) {
#     b.start(K::List);
#     b.bump();
#     while !b.at(K::Close) && !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#     b.eat(K::Close);
#     b.finish();
# }
use incremental_lang::Element;

let doc = Document::new(Lists, "(a) b")?;
for child in doc.tree().root().children() {
    match child {
        Element::Node(node) => assert_eq!(node.kind(), K::List),
        Element::Token(token) => assert!(matches!(token.kind, K::Space | K::Atom)),
    }
}
# Ok::<(), incremental_lang::Error>(())
```

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, List, Open, Close, Atom, Space }
# impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
# struct Lists;
# impl Grammar for Lists {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         let bytes = text.as_bytes();
#         let mut i = 0;
#         while i < bytes.len() {
#             let start = i;
#             let kind = match bytes[i] {
#                 b'(' => { i += 1; K::Open }
#                 b')' => { i += 1; K::Close }
#                 b' ' => { while i < bytes.len() && bytes[i] == b' ' { i += 1; } K::Space }
#                 _ => { while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; } K::Atom }
#             };
#             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
#         }
#     }
#     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
#         if kind == K::List { return list(b); }
#         b.start(K::Root);
#         while !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#         b.finish();
#     }
#     fn is_reparsable(&self, kind: K) -> bool { kind == K::List }
# }
# fn list(b: &mut Builder<'_, K>) {
#     b.start(K::List);
#     b.bump();
#     while !b.at(K::Close) && !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#     b.eat(K::Close);
#     b.finish();
# }
use incremental_lang::Element;

let doc = Document::new(Lists, "(a) b")?;
let tokens: Vec<Span> = doc.tree().root().children().filter_map(Element::as_token).map(|t| t.span).collect();
assert_eq!(tokens, [Span::new(3, 4), Span::new(4, 5)]);
let nodes = doc.tree().root().children().filter_map(Element::as_node).count();
assert_eq!(nodes, 1);
# Ok::<(), incremental_lang::Error>(())
```

---

## `Error`

```rust,ignore
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    OutOfBounds { span: Span, len: u32 },
    NotCharBoundary { offset: u32 },
    TooLarge { len: usize },
    Tokens { offset: u32 },
}
```

Why [`Document::new`](#documentnew) or [`Document::edit`](#documentedit)
failed. A failed edit changes nothing. `Error` implements `Display` and
`core::error::Error`.

| Variant | Meaning | What to do |
|---|---|---|
| `OutOfBounds { span, len }` | The edit's span ends past the text (`len` bytes). | The caller's view of the text has drifted; resynchronize, for example with one edit replacing the whole text. |
| `NotCharBoundary { offset }` | An end of the span splits a UTF-8 character. | Convert positions to byte offsets first — the Language Server Protocol counts UTF-16 code units. |
| `TooLarge { len }` | The text would exceed `u32::MAX` bytes (the 32-bit offsets `Span` uses). | Split the input; 4 GiB is the limit. |
| `Tokens { offset }` | The lexer broke the tiling contract on the whole text; `offset` is the first byte where tokens and text disagree. | Fix the grammar's lexer — this is a defect, not bad input. |

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, List, Open, Close, Atom, Space }
# impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
# struct Lists;
# impl Grammar for Lists {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         let bytes = text.as_bytes();
#         let mut i = 0;
#         while i < bytes.len() {
#             let start = i;
#             let kind = match bytes[i] {
#                 b'(' => { i += 1; K::Open }
#                 b')' => { i += 1; K::Close }
#                 b' ' => { while i < bytes.len() && bytes[i] == b' ' { i += 1; } K::Space }
#                 _ => { while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; } K::Atom }
#             };
#             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
#         }
#     }
#     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
#         if kind == K::List { return list(b); }
#         b.start(K::Root);
#         while !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#         b.finish();
#     }
#     fn is_reparsable(&self, kind: K) -> bool { kind == K::List }
# }
# fn list(b: &mut Builder<'_, K>) {
#     b.start(K::List);
#     b.bump();
#     while !b.at(K::Close) && !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#     b.eat(K::Close);
#     b.finish();
# }
use incremental_lang::Error;

let mut doc = Document::new(Lists, "(a)")?;
match doc.edit(Span::new(2, 10), "") {
    Err(Error::OutOfBounds { span, len }) => {
        assert_eq!((span, len), (Span::new(2, 10), 3));
    }
    other => return Err(format!("unexpected {other:?}").into()),
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

```rust
use incremental_lang::Error;

let message = Error::NotCharBoundary { offset: 7 }.to_string();
assert_eq!(message, "edit boundary at byte 7 falls inside a UTF-8 character");

// It is a standard error, so it boxes and propagates with `?`.
let boxed: Box<dyn std::error::Error> = Box::new(Error::Tokens { offset: 3 });
assert!(boxed.to_string().contains("byte 3"));
```

---

## Re-exports: `Span`, `Token`, `TokenKind`

```rust,ignore
pub use syntax_lang::{Span, Token, TokenKind};
```

The position, token, and kind types the grammar contract is written in,
re-exported from `syntax-lang` (which takes them from `token-lang` and
`span-lang`) so a grammar needs no other dependency and the versions always
match.

- `Span` — a half-open byte range with 32-bit offsets: `Span::new(start, end)`,
  `Span::empty(at)`, `start()`, `end()`, `len()`, `is_empty()`.
- `Token<K>` — a kind and a span, with public `kind` and `span` fields and
  `Token::new(kind, span)`.
- `TokenKind` — the trait a kind implements; `is_trivia` and `is_eof` default
  to `false`.

```rust
use incremental_lang::{Span, Token, TokenKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum K { Word, Space }
impl TokenKind for K {
    fn is_trivia(&self) -> bool { matches!(self, K::Space) }
}

let token = Token::new(K::Space, Span::new(3, 5));
assert!(token.is_trivia());
assert_eq!(token.span.len(), 2);
assert_eq!(token.span.start().to_u32(), 3);
assert!(Span::empty(4).is_empty());
assert!(!K::Word.is_trivia());
```

---

## Feature flags

| Feature | Default | Effect |
|---|---|---|
| `std` | yes | The standard library. Without it the crate is `no_std` and needs only `alloc`, on a target with atomic pointers (the shared tree uses `Arc`). Forwards to `syntax-lang/std`. |

```toml
[dependencies]
incremental-lang = { version = "1", default-features = false }
```

---

## Guide: writing a grammar

A grammar is a lexer and a recursive-descent parser. The steps below take a
small language from nothing to incremental. The complete JSON-with-comments
grammar in
[`examples/common/json.rs`](https://github.com/jamesgober/incremental-lang/blob/main/examples/common/json.rs)
follows the same plan at full size.

**1. One kind type.** List the node kinds and the token kinds in one `enum`,
mark trivia in its `TokenKind` impl, and add an error kind.

**2. A lexer that tiles.** Walk the text and push one token per lexeme, so
that every byte is covered exactly once. Anything unrecognised becomes an
"unknown" token rather than a gap. Unterminated strings and comments end
somewhere definite — the end of the line, the end of the text.

**3. One function per construct.** Each opens a node, consumes its tokens, and
closes it. Loops over a construct's contents test for the closing token before
testing for the end, and recover from junk by wrapping it in an error node and
carrying on, so one malformed element does not end the construct early.

**4. Dispatch in `parse`.** Route `ROOT` to the document loop and each
reparsable kind to its function.

**5. Mark the bracketed kinds reparsable** — and check, with
[`Stats::rejected`](#stats) and a test against fresh parses, that they hold up
under real edits.

```rust
use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};

// 1. Kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum K {
    File, Block, Stmt, Error,
    LBrace, RBrace, Semi, Word, Space,
}
impl TokenKind for K {
    fn is_trivia(&self) -> bool { matches!(self, K::Space) }
}

struct Lang;

impl Grammar for Lang {
    type Kind = K;
    const ROOT: K = K::File;

    // 2. A lexer that tiles: single-character punctuation, runs of spaces,
    //    runs of anything else.
    fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
        let mut chars = text.char_indices().peekable();
        while let Some((start, c)) = chars.next() {
            let kind = match c {
                '{' => K::LBrace,
                '}' => K::RBrace,
                ';' => K::Semi,
                ' ' | '\n' => K::Space,
                _ => K::Word,
            };
            let mut end = start + c.len_utf8();
            if matches!(kind, K::Space | K::Word) {
                while let Some(&(i, next)) = chars.peek() {
                    let same = match kind {
                        K::Space => matches!(next, ' ' | '\n'),
                        _ => !matches!(next, '{' | '}' | ';' | ' ' | '\n'),
                    };
                    if !same { break; }
                    end = i + next.len_utf8();
                    chars.next();
                }
            }
            tokens.push(Token::new(kind, Span::new(start as u32, end as u32)));
        }
    }

    // 4. Dispatch.
    fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
        match kind {
            K::Block => block(b),
            _ => {
                b.start(K::File);
                while !b.at_end() { item(b); }
                b.finish();
            }
        }
    }

    // 5. Blocks are delimited by braces.
    fn is_reparsable(&self, kind: K) -> bool { kind == K::Block }
}

// 3. One function per construct.
fn item(b: &mut Builder<'_, K>) {
    match b.peek() {
        Some(K::LBrace) => block(b),
        Some(K::Word) => stmt(b),
        _ => { b.start(K::Error); b.bump(); b.finish(); }
    }
}

fn block(b: &mut Builder<'_, K>) {
    b.start(K::Block);
    b.bump(); // `{`
    while !b.at(K::RBrace) && !b.at_end() { item(b); }
    b.eat(K::RBrace);
    b.finish();
}

fn stmt(b: &mut Builder<'_, K>) {
    b.start(K::Stmt);
    while b.at(K::Word) { b.bump(); }
    if !b.eat(K::Semi) {
        b.start(K::Error); // missing `;`
        b.finish();
    }
    b.finish();
}

let mut doc = Document::new(Lang, "fn main {\n let x;\n { y; }\n}\n")?;
let edit = doc.edit(Span::new(21, 22), "zz")?; // `y` -> `zz` in the inner block
assert_eq!((edit.kind(), edit.span()), (K::Block, Span::new(19, 26)));
assert_eq!(doc.tree(), Document::new(Lang, doc.text())?.tree());
# Ok::<(), incremental_lang::Error>(())
```

---

## Guide: wiring a document into an editor

**One document per open file.** Create it when the file opens and route every
change through [`edit`](#documentedit). Editors and the Language Server
Protocol report changes as a range plus replacement text; incremental-lang
takes byte offsets, so convert line/column positions (UTF-16 code units, in
LSP's case) to byte offsets first. A change event that carries several edits
is applied in order. When an editor sends the whole text instead, replace
everything with one edit.

**Refresh only what changed.** [`Reparse::span`](#reparse) is the region whose
syntax was rebuilt; semantic highlighting, folding ranges, and outline entries
outside it are still valid, shifted by the edit's change in length.

**Diagnostics from the tree.** If the grammar records errors as nodes,
collecting them is a walk over the tree — and because they are nodes, they are
always in step with the text.

**Analyse on other threads.** Clone the tree (O(1)) and send it, together with
a copy of the text, to worker threads. The worker reads a version that never
changes while the document moves on.

```rust
# use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, List, Open, Close, Atom, Space }
# impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
# struct Lists;
# impl Grammar for Lists {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         let bytes = text.as_bytes();
#         let mut i = 0;
#         while i < bytes.len() {
#             let start = i;
#             let kind = match bytes[i] {
#                 b'(' => { i += 1; K::Open }
#                 b')' => { i += 1; K::Close }
#                 b' ' => { while i < bytes.len() && bytes[i] == b' ' { i += 1; } K::Space }
#                 _ => { while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; } K::Atom }
#             };
#             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
#         }
#     }
#     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
#         if kind == K::List { return list(b); }
#         b.start(K::Root);
#         while !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#         b.finish();
#     }
#     fn is_reparsable(&self, kind: K) -> bool { kind == K::List }
# }
# fn list(b: &mut Builder<'_, K>) {
#     b.start(K::List);
#     b.bump();
#     while !b.at(K::Close) && !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#     b.eat(K::Close);
#     b.finish();
# }
use std::sync::mpsc;
use std::thread;

let mut doc = Document::new(Lists, "(a b)")?;
let (sender, receiver) = mpsc::channel();

let worker = thread::spawn(move || {
    receiver
        .into_iter()
        .map(|(tree, text): (incremental_lang::Tree<K>, String)| {
            // Each version is self-consistent: its tree describes its text.
            assert_eq!(tree.root().text(&text), Some(text.as_str()));
            tree.root().tokens().filter(|t| t.kind == K::Atom).count()
        })
        .collect::<Vec<usize>>()
});

for insertion in [" c", " d", " e"] {
    let end = doc.text().len() as u32 - 1;
    doc.edit(Span::new(end, end), insertion)?;
    if sender.send((doc.tree().clone(), doc.text().to_string())).is_err() {
        break;
    }
}
drop(sender);
let counts = worker.join().map_err(|_| "worker panicked")?;
assert_eq!(counts, [3, 4, 5]);
# Ok::<(), Box<dyn std::error::Error>>(())
```

---

## Stability

As of `1.0.0` the public API is frozen. incremental-lang follows
[Semantic Versioning](https://semver.org/); within the `1.x` series:

- The **surface** will not change in a breaking way:
  [`Grammar`](#grammar) (`Kind`, `ROOT`, `lex`, `parse`, `is_reparsable`),
  [`Builder`](#builder) (`peek`, `nth`, `at`, `at_end`, `bump`, `eat`,
  `start`, `finish`, `checkpoint`, `start_at`), [`Checkpoint`](#checkpoint),
  [`Document`](#document) (`new`, `edit`, `text`, `tree`, `grammar`, `stats`),
  [`Reparse`](#reparse) (`kind`, `span`, `is_full`), [`Stats`](#stats),
  [`Tree`](#tree) (`root`, `to_syntax`, `Clone`, `PartialEq`),
  [`Node`](#node) (`kind`, `span`, `text`, `children`, `descendants`,
  `tokens`), [`Element`](#element) (`kind`, `span`, `as_node`, `as_token`),
  [`Error`](#error), and the [`Span`, `Token`, `TokenKind`](#re-exports-span-token-tokenkind)
  re-exports. New items and new provided methods on `Grammar` are minor
  additions; a breaking change means a new major version.
- The **equivalence guarantee** is the contract: for a grammar that meets the
  [lexer and parser contracts](#grammar), the tree after every successful edit
  equals the tree [`Document::new`](#documentnew) produces from the new text.
  Which node an edit rebuilds is an optimization: [`Reparse::span`](#reparse)
  always covers everything that changed, but how small it is may improve.
- The **grammar contract** will not grow: a grammar that meets it under `1.0`
  meets it under every `1.x`. The [document's checks](#when-a-single-node-is-reparsed)
  may become stricter or smarter, never looser.
- The **builder semantics** are fixed: significant tokens are those that are
  neither trivia nor end-of-input markers; trivia is placed as described in
  [Trivia placement](#trivia-placement); the tree is lossless whatever the
  grammar does; and looking past the end of a node being reparsed refuses that
  reparse.
- **Edits are transactional**: a refused edit, or a panic in grammar code
  caught by the caller, leaves text, tree, and statistics unchanged.
- The **meaning of every `Stats` field** and of every `Error` variant is
  fixed, including `edits == partial + full - 1`. The counts a given sequence
  of edits produces are not: they follow the reparse heuristics, which may
  improve.
- `Stats` and `Error` are `#[non_exhaustive]`, so counters and failure modes
  can be added in a minor release. Read `Stats` fields; match `Error` with a
  wildcard arm.
- `syntax-lang` is a **public dependency**: the re-exported `Span`, `Token`, and
  `TokenKind`, and the return type of `Tree::to_syntax`, come from
  `syntax-lang` 1. Moving to a new major version of it would be a major release
  here.
- MSRV (Rust 1.85) is a compatibility surface: raising it is a documented minor
  change, never a patch.

What is **not** promised: the exact `Display` wording of `Error`, the `Debug`
output of any type, the internal representation of trees, which node a given
edit rebuilds, the `Stats` counts for a given edit sequence, and performance
figures — the benchmarks are tracked, but they are measurements, not
guarantees.

See [`../dev/ROADMAP.md`](../dev/ROADMAP.md) and
[`../CHANGELOG.md`](../CHANGELOG.md).
