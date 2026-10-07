//! [`Document`]: a text, its syntax tree, and the incremental reparse that
//! keeps the two in step.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use syntax_lang::{Span, Token, TokenKind};

use crate::builder::{Builder, Open};
use crate::error::Error;
use crate::grammar::Grammar;
use crate::green::Green;
use crate::stats::{Reparse, Stats};
use crate::tree::Tree;

/// A source text and its concrete syntax tree, kept in step under edits.
///
/// `Document` is the crate's entry point. Create one from a
/// [`Grammar`] and the initial text; then report each change with
/// [`edit`](Self::edit). Each edit updates the text and rebuilds as little of
/// the tree as is safe — usually the innermost reparsable node around the
/// change — and the resulting tree is always identical to what parsing the new
/// text from scratch would produce.
///
/// # Cost of an edit
///
/// An edit that is served by a single node costs a lex and parse of that node,
/// a walk down from the root to find it, and the splice. Every other subtree is
/// reused untouched. When the document is the only owner of its tree, the
/// splice updates the path in place — its cost is the depth of the node, not
/// the size of anything around it. Updating the text itself is a `String`
/// splice: a `memmove` of the text after the edit.
///
/// Finding the node scans each level's children up to the edit, so a list that
/// is very wide at one level (a top-level array of a hundred thousand elements)
/// adds that scan to every edit beneath it; grammars that nest keep it short.
///
/// When single-node attempts fail, the document tries larger nodes, but it
/// never spends more than one whole-document parse's worth of lexing on failed
/// attempts before reparsing everything. An edit therefore never costs more
/// than about twice a full parse.
///
/// # Snapshots and threads
///
/// [`tree`](Self::tree) borrows the current tree; clone it to keep an
/// independent snapshot, which costs one reference-count increment. Snapshots
/// share structure with the live tree and are `Send + Sync` when the kind is,
/// so a language server can hand one to a worker thread and go on editing.
/// While a snapshot is alive, the next edit copies the child arrays on its
/// path instead of updating them in place (copy-on-write), so the snapshot
/// never changes; once the snapshot is dropped, edits are in place again.
///
/// # Examples
///
/// ```
/// use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
///
/// #[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// enum K { Root, List, Open, Close, Atom, Space }
///
/// impl TokenKind for K {
///     fn is_trivia(&self) -> bool { matches!(self, K::Space) }
/// }
///
/// struct Lists;
///
/// impl Grammar for Lists {
///     type Kind = K;
///     const ROOT: K = K::Root;
///
///     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
///         let bytes = text.as_bytes();
///         let mut i = 0;
///         while i < bytes.len() {
///             let start = i;
///             let kind = match bytes[i] {
///                 b'(' => { i += 1; K::Open }
///                 b')' => { i += 1; K::Close }
///                 b' ' => {
///                     while i < bytes.len() && bytes[i] == b' ' { i += 1; }
///                     K::Space
///                 }
///                 _ => {
///                     while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; }
///                     K::Atom
///                 }
///             };
///             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
///         }
///     }
///
///     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
///         if kind == K::List {
///             return list(b);
///         }
///         b.start(K::Root);
///         while !b.at_end() {
///             if b.at(K::Open) { list(b) } else { b.bump() }
///         }
///         b.finish();
///     }
///
///     fn is_reparsable(&self, kind: K) -> bool {
///         kind == K::List
///     }
/// }
///
/// fn list(b: &mut Builder<'_, K>) {
///     b.start(K::List);
///     b.bump();
///     while !b.at(K::Close) && !b.at_end() {
///         if b.at(K::Open) { list(b) } else { b.bump() }
///     }
///     b.eat(K::Close);
///     b.finish();
/// }
///
/// let mut doc = Document::new(Lists, "(let (x 1) (y 2))")?;
///
/// // Change `1` to `10`: only `(x 1)` is relexed and reparsed.
/// let edit = doc.edit(Span::new(8, 9), "10")?;
/// assert_eq!(edit.span(), Span::new(5, 11));
/// assert_eq!(doc.text(), "(let (x 10) (y 2))");
///
/// // The tree always matches a fresh parse of the new text.
/// let fresh = Document::new(Lists, doc.text())?;
/// assert_eq!(doc.tree(), fresh.tree());
/// # Ok::<(), incremental_lang::Error>(())
/// ```
pub struct Document<G: Grammar> {
    grammar: G,
    text: String,
    tree: Tree<G::Kind>,
    stats: Stats,
    scratch: Scratch<G::Kind>,
}

/// Buffers reused across edits so that steady-state editing allocates only
/// the tree nodes it builds.
struct Scratch<K> {
    tokens: Vec<Token<K>>,
    children: Vec<Green<K>>,
    open: Vec<Open<K>>,
    path: Vec<Step<K>>,
    /// The new text of the window being relexed.
    window: String,
}

impl<K> Default for Scratch<K> {
    fn default() -> Self {
        Self {
            tokens: Vec::new(),
            children: Vec::new(),
            open: Vec::new(),
            path: Vec::new(),
            window: String::new(),
        }
    }
}

/// One level of the path from the root down to the innermost node around an
/// edit: the node is child `index` of the previous level's node and starts at
/// byte `start` of the old text.
///
/// Everything the candidate loop needs about the node is recorded on the way
/// down, so trying a candidate never walks the tree again: walking down once
/// per candidate would make an edit beneath a deep chain of nodes quadratic in
/// the depth. The path holds positions and copies, not references — a
/// reference into the tree would keep its child arrays shared, and the splice
/// could no longer update them in place.
#[derive(Clone, Copy)]
struct Step<K> {
    index: usize,
    start: u32,
    width: u32,
    kind: K,
    /// The nearest non-empty tokens before and after the node.
    before: Option<Leaf<K>>,
    after: Option<Leaf<K>>,
    /// The node's own first and last non-empty tokens, recorded only for a
    /// reparsable node (`None` otherwise).
    edges: Option<(Leaf<K>, Leaf<K>)>,
}

/// A leaf token beside or at the edge of a node: its kind and width.
type Leaf<K> = (K, u32);

impl<G: Grammar> Document<G> {
    /// Creates a document from `grammar` and `text`, lexing and parsing the
    /// whole text once.
    ///
    /// # Errors
    ///
    /// - [`Error::TooLarge`] if `text` is longer than `u32::MAX` bytes.
    /// - [`Error::Tokens`] if the grammar's lexer does not tile `text`.
    ///
    /// # Examples
    ///
    /// ```
    /// use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
    ///
    /// #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    /// enum K { Root, Char }
    /// impl TokenKind for K {}
    ///
    /// struct Chars;
    /// impl Grammar for Chars {
    ///     type Kind = K;
    ///     const ROOT: K = K::Root;
    ///     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
    ///         for (i, c) in text.char_indices() {
    ///             let end = (i + c.len_utf8()) as u32;
    ///             tokens.push(Token::new(K::Char, Span::new(i as u32, end)));
    ///         }
    ///     }
    ///     fn parse(&self, _: K, b: &mut Builder<'_, K>) {
    ///         b.start(K::Root);
    ///         while !b.at_end() { b.bump(); }
    ///         b.finish();
    ///     }
    /// }
    ///
    /// let doc = Document::new(Chars, String::from("hi"))?;
    /// assert_eq!(doc.text(), "hi");
    /// assert_eq!(doc.tree().root().tokens().count(), 2);
    ///
    /// // An empty document is fine too: a root with no children.
    /// let empty = Document::new(Chars, "")?;
    /// assert_eq!(empty.tree().root().span(), Span::new(0, 0));
    /// # Ok::<(), incremental_lang::Error>(())
    /// ```
    pub fn new(grammar: G, text: impl Into<String>) -> Result<Self, Error> {
        let text = text.into();
        let len = u32::try_from(text.len()).map_err(|_| Error::TooLarge { len: text.len() })?;
        let mut scratch = Scratch::default();
        let root = parse_whole(&grammar, &text, len, &mut scratch)?;
        Ok(Self {
            grammar,
            text,
            tree: Tree::new(root),
            stats: Stats {
                full: 1,
                bytes: u64::from(len),
                ..Stats::default()
            },
            scratch,
        })
    }

    /// Replaces the bytes in `span` with `text` and brings the tree up to date.
    ///
    /// `span` is a byte range of the current text; an empty span inserts, and
    /// an empty `text` deletes. The returned [`Reparse`] says which part of the
    /// tree was rebuilt. Whatever was rebuilt, the tree afterwards is the tree a
    /// fresh parse of the new text produces.
    ///
    /// # Errors
    ///
    /// On error the document is left exactly as it was — text, tree, and
    /// statistics. The edit is transactional: the text and the tree are
    /// replaced together, only once the new tree is complete, so even a panic
    /// in the grammar's own code (caught by the caller) leaves the document
    /// unchanged and usable.
    ///
    /// - [`Error::OutOfBounds`] if `span` ends past the end of the text.
    /// - [`Error::NotCharBoundary`] if either end of `span` splits a UTF-8
    ///   character.
    /// - [`Error::TooLarge`] if the new text would exceed `u32::MAX` bytes.
    /// - [`Error::Tokens`] if the edit required a whole-document parse and the
    ///   grammar's lexer did not tile the new text.
    ///
    /// # Examples
    ///
    /// Inserting, deleting, and replacing:
    ///
    /// ```
    /// use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
    ///
    /// #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    /// enum K { Root, Char }
    /// impl TokenKind for K {}
    ///
    /// struct Chars;
    /// impl Grammar for Chars {
    ///     type Kind = K;
    ///     const ROOT: K = K::Root;
    ///     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
    ///         for (i, c) in text.char_indices() {
    ///             let end = (i + c.len_utf8()) as u32;
    ///             tokens.push(Token::new(K::Char, Span::new(i as u32, end)));
    ///         }
    ///     }
    ///     fn parse(&self, _: K, b: &mut Builder<'_, K>) {
    ///         b.start(K::Root);
    ///         while !b.at_end() { b.bump(); }
    ///         b.finish();
    ///     }
    /// }
    ///
    /// let mut doc = Document::new(Chars, "hello")?;
    /// doc.edit(Span::new(5, 5), " world")?; // insert
    /// doc.edit(Span::new(0, 1), "J")?;      // replace
    /// doc.edit(Span::new(5, 11), "")?;      // delete
    /// assert_eq!(doc.text(), "Jello");
    /// # Ok::<(), incremental_lang::Error>(())
    /// ```
    ///
    /// A refused edit leaves the document untouched:
    ///
    /// ```
    /// # use incremental_lang::{Builder, Document, Error, Grammar, Span, Token, TokenKind};
    /// # #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    /// # enum K { Root, Char }
    /// # impl TokenKind for K {}
    /// # struct Chars;
    /// # impl Grammar for Chars {
    /// #     type Kind = K;
    /// #     const ROOT: K = K::Root;
    /// #     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
    /// #         for (i, c) in text.char_indices() {
    /// #             tokens.push(Token::new(K::Char, Span::new(i as u32, (i + c.len_utf8()) as u32)));
    /// #         }
    /// #     }
    /// #     fn parse(&self, _: K, b: &mut Builder<'_, K>) {
    /// #         b.start(K::Root);
    /// #         while !b.at_end() { b.bump(); }
    /// #         b.finish();
    /// #     }
    /// # }
    /// let mut doc = Document::new(Chars, "née")?;
    /// // `é` occupies bytes 1..3; byte 2 is inside it.
    /// assert_eq!(
    ///     doc.edit(Span::new(2, 2), "x"),
    ///     Err(Error::NotCharBoundary { offset: 2 }),
    /// );
    /// assert_eq!(doc.text(), "née");
    /// assert_eq!(doc.stats().edits, 0);
    /// # Ok::<(), Error>(())
    /// ```
    pub fn edit(&mut self, span: Span, text: &str) -> Result<Reparse<G::Kind>, Error> {
        let old_len = self.text.len();
        let (start, end) = (span.start().to_usize(), span.end().to_usize());
        if end > old_len {
            return Err(Error::OutOfBounds {
                span,
                len: self.tree.green().width,
            });
        }
        for offset in [start, end] {
            if !self.text.is_char_boundary(offset) {
                return Err(Error::NotCharBoundary {
                    offset: span_offset(offset),
                });
            }
        }
        let new_len = old_len - (end - start) + text.len();
        let new_len = u32::try_from(new_len).map_err(|_| Error::TooLarge { len: new_len })?;

        let edit = Edit {
            start,
            end,
            text,
            delta: i64::from(new_len) - i64::from(self.tree.green().width),
            new_len,
        };
        locate(
            &self.grammar,
            self.tree.green(),
            span_offset(start),
            span_offset(end),
            &mut self.scratch.path,
        );
        let outcome = self.rebuild(&edit);
        self.scratch.path.clear();
        outcome
    }

    /// The current text.
    ///
    /// # Examples
    ///
    /// ```
    /// # use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
    /// # #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    /// # enum K { Root, Char }
    /// # impl TokenKind for K {}
    /// # struct Chars;
    /// # impl Grammar for Chars {
    /// #     type Kind = K;
    /// #     const ROOT: K = K::Root;
    /// #     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
    /// #         for (i, c) in text.char_indices() {
    /// #             tokens.push(Token::new(K::Char, Span::new(i as u32, (i + c.len_utf8()) as u32)));
    /// #         }
    /// #     }
    /// #     fn parse(&self, _: K, b: &mut Builder<'_, K>) {
    /// #         b.start(K::Root);
    /// #         while !b.at_end() { b.bump(); }
    /// #         b.finish();
    /// #     }
    /// # }
    /// let mut doc = Document::new(Chars, "ab")?;
    /// doc.edit(Span::new(1, 1), "-")?;
    /// assert_eq!(doc.text(), "a-b");
    /// # Ok::<(), incremental_lang::Error>(())
    /// ```
    #[inline]
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The current syntax tree. Clone it to keep a snapshot.
    ///
    /// # Examples
    ///
    /// ```
    /// # use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
    /// # #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    /// # enum K { Root, Char }
    /// # impl TokenKind for K {}
    /// # struct Chars;
    /// # impl Grammar for Chars {
    /// #     type Kind = K;
    /// #     const ROOT: K = K::Root;
    /// #     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
    /// #         for (i, c) in text.char_indices() {
    /// #             tokens.push(Token::new(K::Char, Span::new(i as u32, (i + c.len_utf8()) as u32)));
    /// #         }
    /// #     }
    /// #     fn parse(&self, _: K, b: &mut Builder<'_, K>) {
    /// #         b.start(K::Root);
    /// #         while !b.at_end() { b.bump(); }
    /// #         b.finish();
    /// #     }
    /// # }
    /// let mut doc = Document::new(Chars, "ab")?;
    /// let snapshot = doc.tree().clone();
    /// doc.edit(Span::new(0, 2), "xyz")?;
    /// assert_eq!(snapshot.root().span(), Span::new(0, 2));
    /// assert_eq!(doc.tree().root().span(), Span::new(0, 3));
    /// # Ok::<(), incremental_lang::Error>(())
    /// ```
    #[inline]
    #[must_use]
    pub fn tree(&self) -> &Tree<G::Kind> {
        &self.tree
    }

    /// The grammar the document parses with.
    ///
    /// # Examples
    ///
    /// ```
    /// # use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
    /// # #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    /// # enum K { Root, Char }
    /// # impl TokenKind for K {}
    /// struct Chars { name: &'static str }
    /// # impl Grammar for Chars {
    /// #     type Kind = K;
    /// #     const ROOT: K = K::Root;
    /// #     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
    /// #         for (i, c) in text.char_indices() {
    /// #             tokens.push(Token::new(K::Char, Span::new(i as u32, (i + c.len_utf8()) as u32)));
    /// #         }
    /// #     }
    /// #     fn parse(&self, _: K, b: &mut Builder<'_, K>) {
    /// #         b.start(K::Root);
    /// #         while !b.at_end() { b.bump(); }
    /// #         b.finish();
    /// #     }
    /// # }
    /// let doc = Document::new(Chars { name: "chars" }, "")?;
    /// assert_eq!(doc.grammar().name, "chars");
    /// # Ok::<(), incremental_lang::Error>(())
    /// ```
    #[inline]
    #[must_use]
    pub fn grammar(&self) -> &G {
        &self.grammar
    }

    /// Counters describing the work this document has done. See [`Stats`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
    /// # #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    /// # enum K { Root, Char }
    /// # impl TokenKind for K {}
    /// # struct Chars;
    /// # impl Grammar for Chars {
    /// #     type Kind = K;
    /// #     const ROOT: K = K::Root;
    /// #     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
    /// #         for (i, c) in text.char_indices() {
    /// #             tokens.push(Token::new(K::Char, Span::new(i as u32, (i + c.len_utf8()) as u32)));
    /// #         }
    /// #     }
    /// #     fn parse(&self, _: K, b: &mut Builder<'_, K>) {
    /// #         b.start(K::Root);
    /// #         while !b.at_end() { b.bump(); }
    /// #         b.finish();
    /// #     }
    /// # }
    /// let mut doc = Document::new(Chars, "abc")?;
    /// assert_eq!((doc.stats().full, doc.stats().bytes), (1, 3));
    ///
    /// doc.edit(Span::new(3, 3), "d")?; // no reparsable kinds: a full reparse
    /// let stats = doc.stats();
    /// assert_eq!((stats.edits, stats.full, stats.bytes), (1, 2, 7));
    /// # Ok::<(), incremental_lang::Error>(())
    /// ```
    #[inline]
    #[must_use]
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// Applies `edit`: tries the reparsable nodes around it from the innermost
    /// out, within the lexing budget, and falls back to a whole-document parse.
    ///
    /// The edit is transactional. Candidates are relexed from a copy of their
    /// new text, and the document's text and tree are replaced together only
    /// once the new tree is complete — so an error, or a panic in the
    /// grammar's own code, leaves the document as it was.
    fn rebuild(&mut self, edit: &Edit<'_>) -> Result<Reparse<G::Kind>, Error> {
        // Work done on refused attempts is counted only if the edit succeeds:
        // a refused edit, or a panic in grammar code, must leave no trace.
        let mut work = Stats::default();
        let mut budget = u64::from(edit.new_len);
        for depth in (0..self.scratch.path.len()).rev() {
            let step = self.scratch.path[depth];
            let Some(edges) = step.edges else {
                continue; // not reparsable
            };
            let Some(window) = Window::around(&step, edit) else {
                continue;
            };
            let cost = u64::from(window.hi - window.lo);
            if cost > budget {
                break;
            }
            budget -= cost;
            work.bytes += cost;
            let Some(node) = self.reparse_node(&window, edges, edit) else {
                work.rejected += 1;
                continue;
            };
            let path = &self.scratch.path[..=depth];
            if self
                .tree
                .green_mut()
                .graft(path.iter().map(|step| step.index), node, edit.delta)
            {
                self.text.replace_range(edit.start..edit.end, edit.text);
                self.count(work, true);
                return Ok(Reparse::new(window.kind, window.node_span(), false));
            }
            // The path no longer leads to a node. It cannot happen — the path
            // was read from this tree — but a full reparse is the safe answer.
            break;
        }

        let mut text = String::with_capacity(edit.new_len as usize);
        text.push_str(&self.text[..edit.start]);
        text.push_str(edit.text);
        text.push_str(&self.text[edit.end..]);
        let root = parse_whole(&self.grammar, &text, edit.new_len, &mut self.scratch)?;
        self.text = text;
        self.tree = Tree::new(root);
        work.bytes += u64::from(edit.new_len);
        self.count(work, false);
        Ok(Reparse::new(G::ROOT, Span::new(0, edit.new_len), true))
    }

    /// Commits the counters of one successful edit.
    fn count(&mut self, work: Stats, partial: bool) {
        self.stats.edits += 1;
        self.stats.bytes += work.bytes;
        self.stats.rejected += work.rejected;
        if partial {
            self.stats.partial += 1;
        } else {
            self.stats.full += 1;
        }
    }

    /// Relexes and reparses the node at `depth` of the path on its own,
    /// returning the new node only if every safety check passes.
    fn reparse_node(
        &mut self,
        window: &Window<G::Kind>,
        (old_first, old_last): (Leaf<G::Kind>, Leaf<G::Kind>),
        edit: &Edit<'_>,
    ) -> Option<Green<G::Kind>> {
        let Scratch {
            tokens,
            children,
            open,
            window: source,
            ..
        } = &mut self.scratch;

        // The window's new text: the old text up to the edit, the
        // replacement, and the old text after the edit.
        let old_hi = usize::try_from(shift(window.hi, -edit.delta)?).ok()?;
        source.clear();
        source.push_str(self.text.get(window.lo as usize..edit.start)?);
        source.push_str(edit.text);
        source.push_str(self.text.get(edit.end..old_hi)?);

        tokens.clear();
        self.grammar.lex(source, tokens);
        tile(tokens, window.hi - window.lo).ok()?;

        let region = window.strip_neighbours(tokens)?;
        let (first, last) = (region.first()?, region.last()?);
        let same_edges = !first.span.is_empty()
            && !last.span.is_empty()
            && first.kind == old_first.0
            && last.kind == old_last.0;
        if !same_edges {
            return None;
        }

        let mut builder = Builder::new(region, children, open);
        self.grammar.parse(window.kind, &mut builder);
        builder.finish_reparse(window.kind)
    }
}

impl<G: Grammar> fmt::Debug for Document<G>
where
    G::Kind: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Document")
            .field("len", &self.text.len())
            .field("root", &self.tree.root())
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

/// The edit being applied, validated: `start..end` of the old text becomes
/// `text`.
struct Edit<'t> {
    start: usize,
    end: usize,
    text: &'t str,
    /// New length minus old length.
    delta: i64,
    /// Length of the text after the edit.
    new_len: u32,
}

/// The stretch of new text relexed to reparse one node: the node itself plus
/// the token on either side, which must come back unchanged.
struct Window<K> {
    kind: K,
    /// Start of the window: the start of the token before the node, or the
    /// node's start if nothing precedes it.
    lo: u32,
    /// End of the window: the end of the token after the node, or the node's
    /// end if nothing follows it.
    hi: u32,
    /// Where the node starts and ends in the new text.
    node_start: u32,
    node_end: u32,
    prev: Option<Leaf<K>>,
    next: Option<Leaf<K>>,
}

impl<K: TokenKind + Copy + Eq> Window<K> {
    /// The window for the node `step` describes, in the coordinates of the
    /// edited text.
    fn around(step: &Step<K>, edit: &Edit<'_>) -> Option<Self> {
        let node_start = step.start;
        let node_end = shift(node_start.checked_add(step.width)?, edit.delta)?;
        let (prev, next) = (step.before, step.after);
        Some(Self {
            kind: step.kind,
            lo: node_start.checked_sub(prev.map_or(0, |(_, width)| width))?,
            hi: node_end.checked_add(next.map_or(0, |(_, width)| width))?,
            node_start,
            node_end,
            prev,
            next,
        })
    }

    fn node_span(&self) -> Span {
        Span::new(self.node_start, self.node_end)
    }

    /// Checks that the window's lex begins and ends with the same neighbouring
    /// tokens as before, and returns the tokens between them — the node's own.
    fn strip_neighbours<'t>(&self, tokens: &'t [Token<K>]) -> Option<&'t [Token<K>]> {
        let mut region = tokens;
        if let Some(expected) = self.prev {
            let (first, rest) = region.split_first()?;
            if (first.kind, first.span.len()) != expected {
                return None;
            }
            region = rest;
        }
        if let Some(expected) = self.next {
            let (last, rest) = region.split_last()?;
            if (last.kind, last.span.len()) != expected {
                return None;
            }
            region = rest;
        }
        Some(region)
    }
}

/// Lexes and parses the whole of `text` (of length `len`) as one root node.
fn parse_whole<G: Grammar>(
    grammar: &G,
    text: &str,
    len: u32,
    scratch: &mut Scratch<G::Kind>,
) -> Result<Green<G::Kind>, Error> {
    let Scratch {
        tokens,
        children,
        open,
        ..
    } = scratch;
    tokens.clear();
    grammar.lex(text, tokens);
    tile(tokens, len).map_err(|offset| Error::Tokens { offset })?;
    let mut builder = Builder::new(tokens, children, open);
    grammar.parse(G::ROOT, &mut builder);
    let root = builder.finish_root(G::ROOT);
    tokens.clear();
    Ok(root)
}

/// Checks that `tokens` tile `0..len` exactly, after dropping zero-width
/// end-of-input markers. Returns the first offset where they do not.
fn tile<K: TokenKind>(tokens: &mut Vec<Token<K>>, len: u32) -> Result<(), u32> {
    let mut at = 0u32;
    let mut markers = false;
    for token in tokens.iter() {
        if token.span.start().to_u32() != at {
            return Err(at);
        }
        at = token.span.end().to_u32();
        markers |= token.is_eof() && token.span.is_empty();
    }
    if at != len {
        return Err(at);
    }
    if markers {
        tokens.retain(|token| !(token.is_eof() && token.span.is_empty()));
    }
    Ok(())
}

/// Records in `path` every node that strictly contains `start..end` — begins
/// before `start` and ends after `end` — from the root's child downwards,
/// with what the candidate loop needs to know about each.
///
/// Strict containment matters: it guarantees the first and last bytes of
/// every candidate node are untouched by the edit. The tokens beside each
/// node are carried down the walk: a non-empty sibling at a deeper level is
/// always closer to the node than anything found above it.
fn locate<G: Grammar>(
    grammar: &G,
    root: &Green<G::Kind>,
    start: u32,
    end: u32,
    path: &mut Vec<Step<G::Kind>>,
) {
    path.clear();
    let mut parent = root;
    let mut offset = 0u32;
    let mut before = None;
    let mut after = None;
    loop {
        let siblings = parent.children();
        let mut found = None;
        let mut nearest_before = None;
        let mut at = offset;
        for (index, child) in siblings.iter().enumerate() {
            let child_end = at.saturating_add(child.width);
            if at < start && end < child_end {
                if child.is_node() {
                    found = Some((index, at));
                }
                break;
            }
            if child_end > start {
                break;
            }
            if child.width > 0 {
                nearest_before = Some(child);
            }
            at = child_end;
        }
        let Some((index, at)) = found else {
            break;
        };
        let node = &siblings[index];
        if let Some(sibling) = nearest_before {
            before = sibling.last_leaf();
        }
        if let Some(sibling) = siblings[index + 1..].iter().find(|s| s.width > 0) {
            after = sibling.first_leaf();
        }
        let edges = if grammar.is_reparsable(node.kind) {
            node.first_leaf().zip(node.last_leaf())
        } else {
            None
        };
        path.push(Step {
            index,
            start: at,
            width: node.width,
            kind: node.kind,
            before,
            after,
            edges,
        });
        parent = node;
        offset = at;
    }
}

/// `offset + delta`, if the result is a valid offset.
#[inline]
fn shift(offset: u32, delta: i64) -> Option<u32> {
    u32::try_from(i64::from(offset) + delta).ok()
}

/// A `usize` byte offset known to fit in 32 bits (the document length does).
#[inline]
fn span_offset(offset: usize) -> u32 {
    u32::try_from(offset).unwrap_or(u32::MAX)
}
