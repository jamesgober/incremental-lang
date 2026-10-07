//! # incremental_lang
//!
//! Incremental reparsing for editor tooling: keep a source text and its
//! lossless concrete syntax tree in step as the text is edited, rebuilding only
//! the part of the tree around each change.
//!
//! An editor or language server reparses on every keystroke. Parsing the whole
//! file each time scales with the file; incremental reparsing scales with the
//! edit. This crate finds the innermost node around a change that can be
//! reparsed on its own — a block, an object, an argument list — relexes and
//! reparses just that node, and splices the result into a tree that shares
//! every untouched subtree with the previous one. The result is always the
//! tree a fresh parse would produce: every shortcut is checked, and when a
//! check fails the document falls back to a larger node or the whole text.
//!
//! ## The pieces
//!
//! - [`Grammar`] — what a language supplies: its kind type, a lexer, a parser,
//!   and which node kinds may be reparsed on their own.
//! - [`Builder`] — the cursor-and-tree-builder a grammar's parser works with.
//!   It hides trivia from the grammar and threads it into the tree, so the
//!   tree is lossless whatever the grammar does.
//! - [`Document`] — the text and its tree. [`Document::edit`] applies a change
//!   and reports what was rebuilt as a [`Reparse`]; [`Document::stats`] counts
//!   the work done.
//! - [`Tree`], [`Node`], [`Element`] — the syntax tree: an `O(1)`-to-clone,
//!   thread-safe snapshot, walked through positioned, zero-copy views.
//!   [`Tree::to_syntax`] converts it to a [`syntax_lang::Node`] for tools built
//!   on the family's standard CST.
//!
//! ## Example
//!
//! A grammar for nested lists such as `(a (b c))`, in which every list may be
//! reparsed on its own:
//!
//! ```
//! use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
//!
//! // One kind type names both nodes (Root, List) and tokens (the rest).
//! #[derive(Clone, Copy, Debug, PartialEq, Eq)]
//! enum K { Root, List, Open, Close, Atom, Space }
//!
//! impl TokenKind for K {
//!     fn is_trivia(&self) -> bool { matches!(self, K::Space) }
//! }
//!
//! struct Lists;
//!
//! impl Grammar for Lists {
//!     type Kind = K;
//!     const ROOT: K = K::Root;
//!
//!     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
//!         let bytes = text.as_bytes();
//!         let mut i = 0;
//!         while i < bytes.len() {
//!             let start = i;
//!             let kind = match bytes[i] {
//!                 b'(' => { i += 1; K::Open }
//!                 b')' => { i += 1; K::Close }
//!                 b' ' => {
//!                     while i < bytes.len() && bytes[i] == b' ' { i += 1; }
//!                     K::Space
//!                 }
//!                 _ => {
//!                     while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; }
//!                     K::Atom
//!                 }
//!             };
//!             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
//!         }
//!     }
//!
//!     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
//!         if kind == K::List {
//!             return list(b);
//!         }
//!         b.start(K::Root);
//!         while !b.at_end() {
//!             if b.at(K::Open) { list(b) } else { b.bump() }
//!         }
//!         b.finish();
//!     }
//!
//!     fn is_reparsable(&self, kind: K) -> bool {
//!         kind == K::List
//!     }
//! }
//!
//! fn list(b: &mut Builder<'_, K>) {
//!     b.start(K::List);
//!     b.bump(); // `(`
//!     // Test for the closer before the end, so a balanced list never looks
//!     // past its own `)`.
//!     while !b.at(K::Close) && !b.at_end() {
//!         if b.at(K::Open) { list(b) } else { b.bump() }
//!     }
//!     b.eat(K::Close);
//!     b.finish();
//! }
//!
//! let mut doc = Document::new(Lists, "(add (mul 2 3) 4)")?;
//!
//! // Typing inside `(mul 2 3)` rebuilds only that list.
//! let edit = doc.edit(Span::new(12, 13), "30")?;
//! assert!(!edit.is_full());
//! assert_eq!(edit.span(), Span::new(5, 15));
//! assert_eq!(doc.text(), "(add (mul 2 30) 4)");
//!
//! // Removing a `)` unbalances the text, and the document reparses it all.
//! let edit = doc.edit(Span::new(14, 15), "")?;
//! assert!(edit.is_full());
//!
//! // Either way, the tree matches a parse from scratch.
//! assert_eq!(doc.tree(), Document::new(Lists, doc.text())?.tree());
//! # Ok::<(), incremental_lang::Error>(())
//! ```
//!
//! ## How a reparse is checked
//!
//! A single node is reparsed only if the edit lies strictly inside it, the
//! tokens on either side of it relex unchanged, it still begins and ends with
//! the same kinds of token, and the grammar rebuilds exactly one node of the
//! same kind without looking past the node's last token. The last check is the
//! one that catches unbalanced brackets: a list whose `)` was deleted keeps
//! asking for tokens past its end. Two further rules are the grammar's to keep,
//! because a single reparse cannot observe them: the lexer looks at most one
//! character past each token, and nothing outside a reparsable node depends on
//! its contents beyond its first token. The full contract is set out under
//! [`Grammar`].
//!
//! ## Features
//!
//! - `std` (default) — the standard library. Without it the crate is `no_std`
//!   and needs only `alloc` (and a target with atomic pointers, for the shared
//!   tree). Forwards to `syntax-lang/std`.
//!
//! ## Stability
//!
//! The public surface is frozen and stable as of `1.0.0`: it follows Semantic
//! Versioning, with no breaking changes before `2.0`. The full surface, the
//! guarantees that are part of the contract — above all, that the tree after
//! every edit equals a fresh parse for a grammar that meets the [`Grammar`]
//! contract — and the SemVer promise are catalogued in
//! [`docs/API.md`](https://github.com/jamesgober/incremental-lang/blob/main/docs/API.md#stability).

#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![forbid(unsafe_code)]
#![deny(
    warnings,
    missing_docs,
    unsafe_op_in_unsafe_fn,
    unused_must_use,
    unused_results,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::todo,
    clippy::unimplemented,
    clippy::unreachable,
    clippy::dbg_macro,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::undocumented_unsafe_blocks
)]

extern crate alloc;

mod builder;
mod document;
mod error;
mod grammar;
mod green;
mod stats;
mod tree;

pub use builder::{Builder, Checkpoint};
pub use document::Document;
pub use error::Error;
pub use grammar::Grammar;
pub use stats::{Reparse, Stats};
pub use tree::{Element, Node, Tree};

// Re-exported so a grammar can name the token, kind, and span types this
// crate's API is built on without depending on `syntax-lang` (and through it
// `token-lang` and `span-lang`) directly — and so the versions always match.
pub use syntax_lang::{Span, Token, TokenKind};

/// Compiles and runs the `rust` code blocks in `README.md` and `docs/API.md` as
/// part of `cargo test`, so the published examples cannot drift from the API.
///
/// Present only while collecting doctests (`#[cfg(doctest)]`); it is not part of
/// the public surface and does not appear in the built library or its docs.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
#[doc = include_str!("../docs/API.md")]
pub struct MarkdownDocTests;
