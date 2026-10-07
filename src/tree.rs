//! The public view of a document's syntax: [`Tree`], [`Node`], and
//! [`Element`].

use alloc::vec::Vec;
use core::{fmt, slice};

use syntax_lang::{Span, Token};

use crate::green::Green;

/// An immutable snapshot of a document's concrete syntax tree.
///
/// A tree is *lossless*: every byte of the source belongs to exactly one token,
/// trivia included, so the tokens of any node reproduce its text exactly. It
/// is also *cheap to keep*: cloning a tree is one reference-count increment,
/// the snapshot never changes, and it can be sent to another thread
/// (`Tree<K>` is `Send + Sync` when `K` is) while the
/// [`Document`](crate::Document) it came from goes on being edited. Successive
/// snapshots share every subtree an edit did not touch.
///
/// A tree stores widths, not positions, and holds no text. Positions are
/// computed as you walk down from [`root`](Self::root); text is borrowed from
/// the source string you pass to [`Node::text`].
///
/// Equality is structural — same kinds, same spans, same shape — and is how
/// the property tests check that an incremental reparse matches a fresh parse.
/// It does not compare text: a tree holds none, so renaming `a` to `b`
/// produces an equal tree. Compare the source strings as well when that
/// matters.
///
/// # Examples
///
/// ```
/// use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
/// # #[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// # enum K { Root, List, Open, Close, Atom, Space }
/// # impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
/// # struct Lists;
/// # impl Grammar for Lists {
/// #     type Kind = K;
/// #     const ROOT: K = K::Root;
/// #     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
/// #         let bytes = text.as_bytes();
/// #         let mut i = 0;
/// #         while i < bytes.len() {
/// #             let start = i;
/// #             let kind = match bytes[i] {
/// #                 b'(' => { i += 1; K::Open }
/// #                 b')' => { i += 1; K::Close }
/// #                 b' ' => { while i < bytes.len() && bytes[i] == b' ' { i += 1; } K::Space }
/// #                 _ => { while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; } K::Atom }
/// #             };
/// #             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
/// #         }
/// #     }
/// #     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
/// #         if kind == K::List { return list(b); }
/// #         b.start(K::Root);
/// #         while !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
/// #         b.finish();
/// #     }
/// #     fn is_reparsable(&self, kind: K) -> bool { kind == K::List }
/// # }
/// # fn list(b: &mut Builder<'_, K>) {
/// #     b.start(K::List);
/// #     b.bump();
/// #     while !b.at(K::Close) && !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
/// #     b.eat(K::Close);
/// #     b.finish();
/// # }
/// let mut doc = Document::new(Lists, "(a b) (c)")?;
/// let before = doc.tree().clone(); // O(1) snapshot
///
/// doc.edit(Span::new(7, 8), "c d")?;
///
/// // The snapshot still describes the old text.
/// assert_eq!(before.root().span(), Span::new(0, 9));
/// assert_eq!(doc.tree().root().span(), Span::new(0, 11));
/// assert_ne!(&before, doc.tree());
///
/// // Undoing the edit restores an equal tree. Equality compares kinds and
/// // spans, not text: renaming `c` to `z` would also compare equal.
/// doc.edit(Span::new(7, 10), "c")?;
/// assert_eq!(&before, doc.tree());
/// # Ok::<(), incremental_lang::Error>(())
/// ```
#[derive(Clone)]
pub struct Tree<K> {
    root: Green<K>,
}

impl<K> Tree<K> {
    pub(crate) const fn new(root: Green<K>) -> Self {
        Self { root }
    }

    pub(crate) const fn green(&self) -> &Green<K> {
        &self.root
    }

    pub(crate) fn green_mut(&mut self) -> &mut Green<K> {
        &mut self.root
    }
}

impl<K: Copy> Tree<K> {
    /// The root node, spanning the whole document.
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
    /// let doc = Document::new(Chars, "hello")?;
    /// let root = doc.tree().root();
    /// assert_eq!(root.kind(), K::Root);
    /// assert_eq!(root.span(), Span::new(0, 5));
    /// # Ok::<(), incremental_lang::Error>(())
    /// ```
    #[inline]
    #[must_use]
    pub fn root(&self) -> Node<'_, K> {
        Node {
            green: &self.root,
            start: 0,
        }
    }

    /// Converts the tree into a [`syntax_lang::Node`], the `-lang` family's
    /// standard concrete syntax tree, with absolute spans.
    ///
    /// Use it to hand a document to tooling built on `syntax-lang` — a
    /// formatter, a tree-sitter bridge. The conversion copies the whole tree,
    /// so it costs time and memory in proportion to the document; it is meant
    /// for handing off a finished snapshot, not for reading the tree after
    /// every keystroke (walk [`root`](Self::root) for that). It is iterative
    /// and safe on trees of any depth.
    ///
    /// # Examples
    ///
    /// ```
    /// use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
    ///
    /// #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    /// enum K { Root, Word, Space }
    /// impl TokenKind for K {
    ///     fn is_trivia(&self) -> bool { matches!(self, K::Space) }
    /// }
    ///
    /// struct Words;
    /// impl Grammar for Words {
    ///     type Kind = K;
    ///     const ROOT: K = K::Root;
    ///     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
    ///         for (i, byte) in text.bytes().enumerate() {
    ///             let kind = if byte == b' ' { K::Space } else { K::Word };
    ///             tokens.push(Token::new(kind, Span::new(i as u32, i as u32 + 1)));
    ///         }
    ///     }
    ///     fn parse(&self, _: K, b: &mut Builder<'_, K>) {
    ///         b.start(K::Root);
    ///         while !b.at_end() { b.bump(); }
    ///         b.finish();
    ///     }
    /// }
    ///
    /// let doc = Document::new(Words, "a b")?;
    /// let cst = doc.tree().to_syntax();
    /// assert_eq!(cst.kind(), &K::Root);
    /// assert_eq!(cst.text(doc.text()), Some("a b"));
    /// assert_eq!(cst.tokens().filter(|t| !t.is_trivia()).count(), 2);
    /// # Ok::<(), incremental_lang::Error>(())
    /// ```
    #[must_use]
    pub fn to_syntax(&self) -> syntax_lang::Node<K> {
        let mut builder = syntax_lang::Builder::new();
        for event in Walk::new(self.root()) {
            match event {
                Event::Enter(node) => builder.start_node(node.kind()),
                Event::Token(token) => builder.token(token),
                Event::Leave => builder.finish_node(),
            }
        }
        // The walk emits a balanced sequence that opens with the root, so the
        // builder has nothing to report. The fallback exists only so that this
        // conversion can never panic.
        builder
            .finish()
            .unwrap_or_else(|_| syntax_lang::Node::new(self.root.kind, Vec::new()))
    }
}

impl<K: PartialEq> PartialEq for Tree<K> {
    fn eq(&self, other: &Self) -> bool {
        self.root.same(&other.root)
    }
}

impl<K: Eq> Eq for Tree<K> {}

impl<K: Copy + fmt::Debug> fmt::Debug for Tree<K> {
    /// Writes the tree as an indented outline, one node or token per line:
    ///
    /// ```text
    /// Root@0..5
    ///   List@0..5
    ///     Open@0..1
    ///     Atom@1..4
    ///     Close@4..5
    /// ```
    ///
    /// The format is for people; it is not a stable interface.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut depth = 0usize;
        for event in Walk::new(self.root()) {
            let (kind, span) = match event {
                Event::Leave => {
                    depth = depth.saturating_sub(1);
                    continue;
                }
                Event::Enter(node) => (node.kind(), node.span()),
                Event::Token(token) => (token.kind, token.span),
            };
            writeln!(
                f,
                "{:indent$}{kind:?}@{}..{}",
                "",
                span.start().to_u32(),
                span.end().to_u32(),
                indent = depth * 2
            )?;
            if matches!(event, Event::Enter(_)) {
                depth += 1;
            }
        }
        Ok(())
    }
}

/// A node of a [`Tree`], positioned in the document.
///
/// A `Node` is a cheap, `Copy` view — a reference into the tree plus the
/// node's absolute start — so its methods take `self` by value and the
/// iterators they return borrow only the tree.
///
/// # Examples
///
/// ```
/// use incremental_lang::{Builder, Document, Element, Grammar, Span, Token, TokenKind};
///
/// #[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// enum K { Root, Pair, Word, Eq }
/// impl TokenKind for K {}
///
/// struct Pairs;
/// impl Grammar for Pairs {
///     type Kind = K;
///     const ROOT: K = K::Root;
///     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
///         for (i, byte) in text.bytes().enumerate() {
///             let kind = if byte == b'=' { K::Eq } else { K::Word };
///             tokens.push(Token::new(kind, Span::new(i as u32, i as u32 + 1)));
///         }
///     }
///     fn parse(&self, _: K, b: &mut Builder<'_, K>) {
///         b.start(K::Root);
///         b.start(K::Pair);
///         while !b.at_end() { b.bump(); }
///         b.finish();
///         b.finish();
///     }
/// }
///
/// let doc = Document::new(Pairs, "k=v")?;
/// let pair = doc.tree().root().descendants().nth(1).ok_or("no pair")?;
/// assert_eq!(pair.kind(), K::Pair);
/// assert_eq!(pair.text(doc.text()), Some("k=v"));
/// let middle = pair.children().nth(1).and_then(Element::as_token).ok_or("no token")?;
/// assert_eq!(middle.kind, K::Eq);
/// assert_eq!(middle.span, Span::new(1, 2));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct Node<'a, K> {
    green: &'a Green<K>,
    start: u32,
}

impl<K> Clone for Node<'_, K> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<K> Copy for Node<'_, K> {}

impl<'a, K: Copy> Node<'a, K> {
    /// The node's kind.
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
    /// let doc = Document::new(Chars, "x")?;
    /// assert_eq!(doc.tree().root().kind(), K::Root);
    /// # Ok::<(), incremental_lang::Error>(())
    /// ```
    #[inline]
    #[must_use]
    pub fn kind(self) -> K {
        self.green.kind
    }

    /// The bytes of the document this node covers.
    ///
    /// Except for the root, a node's span begins on its first significant
    /// token and ends on its last: trivia around a node belongs to its parent.
    /// A node with no tokens has an empty span where it was opened.
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
    /// let doc = Document::new(Chars, "héllo")?;
    /// assert_eq!(doc.tree().root().span(), Span::new(0, 6)); // `é` is two bytes
    /// # Ok::<(), incremental_lang::Error>(())
    /// ```
    #[inline]
    #[must_use]
    pub fn span(self) -> Span {
        Span::new(self.start, self.start.saturating_add(self.green.width))
    }

    /// The node's text, sliced from `source` — zero-copy. Returns `None` if the
    /// span does not fall on character boundaries inside `source`, which means
    /// `source` is not the text the tree was built from.
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
    /// let doc = Document::new(Chars, "abc")?;
    /// let root = doc.tree().root();
    /// assert_eq!(root.text(doc.text()), Some("abc"));
    /// assert_eq!(root.text("ab"), None); // not the document's text
    /// # Ok::<(), incremental_lang::Error>(())
    /// ```
    #[inline]
    #[must_use]
    pub fn text(self, source: &str) -> Option<&str> {
        let span = self.span();
        source.get(span.start().to_usize()..span.end().to_usize())
    }

    /// The node's direct children — nodes and tokens in source order, with
    /// absolute spans.
    ///
    /// # Examples
    ///
    /// ```
    /// # use incremental_lang::{Builder, Document, Element, Grammar, Span, Token, TokenKind};
    /// # #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    /// # enum K { Root, Group, Char }
    /// # impl TokenKind for K {}
    /// # struct Groups;
    /// # impl Grammar for Groups {
    /// #     type Kind = K;
    /// #     const ROOT: K = K::Root;
    /// #     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
    /// #         for (i, c) in text.char_indices() {
    /// #             tokens.push(Token::new(K::Char, Span::new(i as u32, (i + c.len_utf8()) as u32)));
    /// #         }
    /// #     }
    /// #     fn parse(&self, _: K, b: &mut Builder<'_, K>) {
    /// #         b.start(K::Root);
    /// #         b.bump();
    /// #         b.start(K::Group);
    /// #         b.bump();
    /// #         b.bump();
    /// #         b.finish();
    /// #         b.finish();
    /// #     }
    /// # }
    /// // Root holds the token `a` and a Group over `bc`.
    /// let doc = Document::new(Groups, "abc")?;
    /// let children: Vec<(K, Span)> = doc
    ///     .tree()
    ///     .root()
    ///     .children()
    ///     .map(|child| (child.kind(), child.span()))
    ///     .collect();
    /// assert_eq!(children, [(K::Char, Span::new(0, 1)), (K::Group, Span::new(1, 3))]);
    /// # Ok::<(), incremental_lang::Error>(())
    /// ```
    #[inline]
    pub fn children(self) -> impl ExactSizeIterator<Item = Element<'a, K>> {
        Children {
            iter: self.green.children().iter(),
            offset: self.start,
        }
    }

    /// This node and every node beneath it, in pre-order: a parent before its
    /// children, siblings in source order.
    ///
    /// The walk is iterative, so trees of any depth are safe.
    ///
    /// # Examples
    ///
    /// ```
    /// # use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
    /// # #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    /// # enum K { Root, Outer, Inner, Char }
    /// # impl TokenKind for K {}
    /// # struct Nest;
    /// # impl Grammar for Nest {
    /// #     type Kind = K;
    /// #     const ROOT: K = K::Root;
    /// #     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
    /// #         for (i, c) in text.char_indices() {
    /// #             tokens.push(Token::new(K::Char, Span::new(i as u32, (i + c.len_utf8()) as u32)));
    /// #         }
    /// #     }
    /// #     fn parse(&self, _: K, b: &mut Builder<'_, K>) {
    /// #         b.start(K::Root);
    /// #         b.start(K::Outer);
    /// #         b.start(K::Inner);
    /// #         b.bump();
    /// #         b.finish();
    /// #         b.finish();
    /// #         b.finish();
    /// #     }
    /// # }
    /// let doc = Document::new(Nest, "x")?;
    /// let kinds: Vec<K> = doc.tree().root().descendants().map(|n| n.kind()).collect();
    /// assert_eq!(kinds, [K::Root, K::Outer, K::Inner]);
    /// # Ok::<(), incremental_lang::Error>(())
    /// ```
    pub fn descendants(self) -> impl Iterator<Item = Node<'a, K>> {
        Walk::new(self).filter_map(|event| match event {
            Event::Enter(node) => Some(node),
            Event::Token(_) | Event::Leave => None,
        })
    }

    /// Every token beneath this node, in source order — trivia included — with
    /// absolute spans. Concatenating their text reproduces the node's text.
    ///
    /// The walk is iterative, so trees of any depth are safe.
    ///
    /// # Examples
    ///
    /// ```
    /// # use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
    /// # #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    /// # enum K { Root, Word, Space }
    /// # impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
    /// # struct Words;
    /// # impl Grammar for Words {
    /// #     type Kind = K;
    /// #     const ROOT: K = K::Root;
    /// #     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
    /// #         for (i, byte) in text.bytes().enumerate() {
    /// #             let kind = if byte == b' ' { K::Space } else { K::Word };
    /// #             tokens.push(Token::new(kind, Span::new(i as u32, i as u32 + 1)));
    /// #         }
    /// #     }
    /// #     fn parse(&self, _: K, b: &mut Builder<'_, K>) {
    /// #         b.start(K::Root);
    /// #         while !b.at_end() { b.bump(); }
    /// #         b.finish();
    /// #     }
    /// # }
    /// let doc = Document::new(Words, "a b")?;
    /// let rebuilt: String = doc
    ///     .tree()
    ///     .root()
    ///     .tokens()
    ///     .map(|t| &doc.text()[t.span.start().to_usize()..t.span.end().to_usize()])
    ///     .collect();
    /// assert_eq!(rebuilt, "a b");
    /// # Ok::<(), incremental_lang::Error>(())
    /// ```
    pub fn tokens(self) -> impl Iterator<Item = Token<K>> {
        Walk::new(self).filter_map(|event| match event {
            Event::Token(token) => Some(token),
            Event::Enter(_) | Event::Leave => None,
        })
    }
}

impl<K: Copy + fmt::Debug> fmt::Debug for Node<'_, K> {
    /// `Kind@start..end`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let span = self.span();
        write!(
            f,
            "{:?}@{}..{}",
            self.kind(),
            span.start().to_u32(),
            span.end().to_u32()
        )
    }
}

/// One child of a [`Node`]: a nested node or a leaf token.
///
/// # Examples
///
/// ```
/// # use incremental_lang::{Builder, Document, Element, Grammar, Span, Token, TokenKind};
/// # #[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// # enum K { Root, Group, Char }
/// # impl TokenKind for K {}
/// # struct Groups;
/// # impl Grammar for Groups {
/// #     type Kind = K;
/// #     const ROOT: K = K::Root;
/// #     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
/// #         for (i, c) in text.char_indices() {
/// #             tokens.push(Token::new(K::Char, Span::new(i as u32, (i + c.len_utf8()) as u32)));
/// #         }
/// #     }
/// #     fn parse(&self, _: K, b: &mut Builder<'_, K>) {
/// #         b.start(K::Root);
/// #         b.bump();
/// #         b.start(K::Group);
/// #         b.bump();
/// #         b.finish();
/// #         b.finish();
/// #     }
/// # }
/// let doc = Document::new(Groups, "ab")?;
/// for child in doc.tree().root().children() {
///     match child {
///         Element::Token(token) => assert_eq!(token.span, Span::new(0, 1)),
///         Element::Node(node) => assert_eq!(node.kind(), K::Group),
///     }
/// }
/// # Ok::<(), incremental_lang::Error>(())
/// ```
pub enum Element<'a, K> {
    /// A nested node.
    Node(Node<'a, K>),
    /// A leaf token, with its absolute span.
    Token(Token<K>),
}

impl<K: Copy> Clone for Element<'_, K> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<K: Copy> Copy for Element<'_, K> {}

impl<'a, K: Copy> Element<'a, K> {
    /// The child's kind, whether it is a node or a token.
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
    /// let doc = Document::new(Chars, "ab")?;
    /// assert!(doc.tree().root().children().all(|c| c.kind() == K::Char));
    /// # Ok::<(), incremental_lang::Error>(())
    /// ```
    #[inline]
    #[must_use]
    pub fn kind(self) -> K {
        match self {
            Element::Node(node) => node.kind(),
            Element::Token(token) => token.kind,
        }
    }

    /// The bytes of the document the child covers.
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
    /// let doc = Document::new(Chars, "ab")?;
    /// let spans: Vec<Span> = doc.tree().root().children().map(|c| c.span()).collect();
    /// assert_eq!(spans, [Span::new(0, 1), Span::new(1, 2)]);
    /// # Ok::<(), incremental_lang::Error>(())
    /// ```
    #[inline]
    #[must_use]
    pub fn span(self) -> Span {
        match self {
            Element::Node(node) => node.span(),
            Element::Token(token) => token.span,
        }
    }

    /// The nested node, if this child is one.
    ///
    /// # Examples
    ///
    /// ```
    /// # use incremental_lang::{Builder, Document, Element, Grammar, Span, Token, TokenKind};
    /// # #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    /// # enum K { Root, Group, Char }
    /// # impl TokenKind for K {}
    /// # struct Groups;
    /// # impl Grammar for Groups {
    /// #     type Kind = K;
    /// #     const ROOT: K = K::Root;
    /// #     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
    /// #         for (i, c) in text.char_indices() {
    /// #             tokens.push(Token::new(K::Char, Span::new(i as u32, (i + c.len_utf8()) as u32)));
    /// #         }
    /// #     }
    /// #     fn parse(&self, _: K, b: &mut Builder<'_, K>) {
    /// #         b.start(K::Root);
    /// #         b.bump();
    /// #         b.start(K::Group);
    /// #         b.bump();
    /// #         b.finish();
    /// #         b.finish();
    /// #     }
    /// # }
    /// let doc = Document::new(Groups, "ab")?;
    /// let groups: Vec<K> = doc
    ///     .tree()
    ///     .root()
    ///     .children()
    ///     .filter_map(Element::as_node)
    ///     .map(|n| n.kind())
    ///     .collect();
    /// assert_eq!(groups, [K::Group]);
    /// # Ok::<(), incremental_lang::Error>(())
    /// ```
    #[inline]
    #[must_use]
    pub fn as_node(self) -> Option<Node<'a, K>> {
        match self {
            Element::Node(node) => Some(node),
            Element::Token(_) => None,
        }
    }

    /// The leaf token, if this child is one.
    ///
    /// # Examples
    ///
    /// ```
    /// # use incremental_lang::{Builder, Document, Element, Grammar, Span, Token, TokenKind};
    /// # #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    /// # enum K { Root, Group, Char }
    /// # impl TokenKind for K {}
    /// # struct Groups;
    /// # impl Grammar for Groups {
    /// #     type Kind = K;
    /// #     const ROOT: K = K::Root;
    /// #     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
    /// #         for (i, c) in text.char_indices() {
    /// #             tokens.push(Token::new(K::Char, Span::new(i as u32, (i + c.len_utf8()) as u32)));
    /// #         }
    /// #     }
    /// #     fn parse(&self, _: K, b: &mut Builder<'_, K>) {
    /// #         b.start(K::Root);
    /// #         b.bump();
    /// #         b.start(K::Group);
    /// #         b.bump();
    /// #         b.finish();
    /// #         b.finish();
    /// #     }
    /// # }
    /// let doc = Document::new(Groups, "ab")?;
    /// let direct: Vec<Span> = doc
    ///     .tree()
    ///     .root()
    ///     .children()
    ///     .filter_map(Element::as_token)
    ///     .map(|t| t.span)
    ///     .collect();
    /// assert_eq!(direct, [Span::new(0, 1)]); // `b` is inside Group
    /// # Ok::<(), incremental_lang::Error>(())
    /// ```
    #[inline]
    #[must_use]
    pub fn as_token(self) -> Option<Token<K>> {
        match self {
            Element::Token(token) => Some(token),
            Element::Node(_) => None,
        }
    }
}

impl<K: Copy + fmt::Debug> fmt::Debug for Element<'_, K> {
    /// `Kind@start..end`, for nodes and tokens alike.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Element::Node(node) => fmt::Debug::fmt(node, f),
            Element::Token(token) => write!(
                f,
                "{:?}@{}..{}",
                token.kind,
                token.span.start().to_u32(),
                token.span.end().to_u32()
            ),
        }
    }
}

/// The direct children of a node, positioned by a running offset.
struct Children<'a, K> {
    iter: slice::Iter<'a, Green<K>>,
    offset: u32,
}

impl<'a, K: Copy> Iterator for Children<'a, K> {
    type Item = Element<'a, K>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let green = self.iter.next()?;
        let start = self.offset;
        self.offset = start.saturating_add(green.width);
        Some(element(green, start))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl<K: Copy> ExactSizeIterator for Children<'_, K> {}

/// The public view of one green element at absolute offset `start`.
#[inline]
fn element<K: Copy>(green: &Green<K>, start: u32) -> Element<'_, K> {
    if green.is_node() {
        Element::Node(Node { green, start })
    } else {
        Element::Token(Token::new(
            green.kind,
            Span::new(start, start.saturating_add(green.width)),
        ))
    }
}

/// One step of a depth-first walk.
enum Event<'a, K> {
    /// Entering a node; its children follow, then a matching `Leave`.
    Enter(Node<'a, K>),
    /// A leaf token.
    Token(Token<K>),
    /// Leaving the most recently entered node.
    Leave,
}

/// A depth-first walk over a subtree, emitting enter/token/leave events. The
/// stack lives on the heap, so depth is limited by memory, not the call stack.
struct Walk<'a, K> {
    root: Option<Node<'a, K>>,
    stack: Vec<Children<'a, K>>,
}

impl<'a, K> Walk<'a, K> {
    fn new(root: Node<'a, K>) -> Self {
        Self {
            root: Some(root),
            stack: Vec::new(),
        }
    }
}

impl<'a, K: Copy> Iterator for Walk<'a, K> {
    type Item = Event<'a, K>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(root) = self.root.take() {
            self.stack.push(Children {
                iter: root.green.children().iter(),
                offset: root.start,
            });
            return Some(Event::Enter(root));
        }
        let top = self.stack.last_mut()?;
        let Some(child) = top.next() else {
            let _ = self.stack.pop();
            return Some(Event::Leave);
        };
        Some(match child {
            Element::Node(node) => {
                self.stack.push(Children {
                    iter: node.green.children().iter(),
                    offset: node.start,
                });
                Event::Enter(node)
            }
            Element::Token(token) => Event::Token(token),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;
    use alloc::vec;

    fn node(kind: u8, children: Vec<Green<u8>>) -> Green<u8> {
        Green::node(kind, children.into())
    }

    /// Root(0) over [tok 1 (2 bytes), Node 2 over [tok 3 (1), tok 4 (3)]].
    fn sample() -> Tree<u8> {
        Tree::new(node(
            0,
            vec![
                Green::token(1, 2),
                node(2, vec![Green::token(3, 1), Green::token(4, 3)]),
            ],
        ))
    }

    #[test]
    fn test_root_spans_whole_tree() {
        let tree = sample();
        assert_eq!(tree.root().span(), Span::new(0, 6));
        assert_eq!(tree.root().kind(), 0);
    }

    #[test]
    fn test_children_have_absolute_spans() {
        let tree = sample();
        let spans: Vec<(u8, Span)> = tree
            .root()
            .children()
            .map(|c| (c.kind(), c.span()))
            .collect();
        assert_eq!(spans, [(1, Span::new(0, 2)), (2, Span::new(2, 6))]);
        assert_eq!(tree.root().children().len(), 2);
    }

    #[test]
    fn test_descendants_and_tokens_are_ordered() {
        let tree = sample();
        let nodes: Vec<u8> = tree.root().descendants().map(Node::kind).collect();
        assert_eq!(nodes, [0, 2]);
        let tokens: Vec<(u8, Span)> = tree.root().tokens().map(|t| (t.kind, t.span)).collect();
        assert_eq!(
            tokens,
            [
                (1, Span::new(0, 2)),
                (3, Span::new(2, 3)),
                (4, Span::new(3, 6))
            ]
        );
    }

    #[test]
    fn test_text_rejects_mismatched_source() {
        let tree = sample();
        assert_eq!(tree.root().text("abcdef"), Some("abcdef"));
        assert_eq!(tree.root().text("abc"), None);
    }

    #[test]
    fn test_element_accessors() {
        let tree = sample();
        let mut children = tree.root().children();
        let first = children.next();
        assert!(first.and_then(Element::as_token).is_some());
        assert!(first.and_then(Element::as_node).is_none());
        let second = children.next();
        assert!(second.and_then(Element::as_node).is_some());
        assert!(second.and_then(Element::as_token).is_none());
    }

    #[test]
    fn test_debug_outline() {
        let text = format!("{:?}", sample());
        assert_eq!(text, "0@0..6\n  1@0..2\n  2@2..6\n    3@2..3\n    4@3..6\n");
        let child = sample();
        let shown = format!("{:?}", child.root().children().nth(1));
        assert_eq!(shown, "Some(2@2..6)");
    }

    #[test]
    fn test_equality_is_structural() {
        assert_eq!(sample(), sample());
        let other = Tree::new(node(0, vec![Green::token(1, 6)]));
        assert_ne!(sample(), other);
    }

    #[test]
    fn test_to_syntax_preserves_kinds_spans_and_empty_nodes() {
        // Root over [tok(2), empty node 5, tok(1)]
        let tree = Tree::new(node(
            0,
            vec![Green::token(1, 2), node(5, vec![]), Green::token(1, 1)],
        ));
        let cst = tree.to_syntax();
        assert_eq!(cst.span(), Span::new(0, 3));
        let empty = cst.descendants().find(|n| *n.kind() == 5);
        assert_eq!(empty.map(syntax_lang::Node::span), Some(Span::empty(2)));
        assert_eq!(cst.tokens().count(), 2);
    }

    #[test]
    fn test_deep_tree_walks_iteratively() {
        let mut green = node(1, vec![Green::token(2, 1)]);
        for _ in 0..100_000 {
            green = node(1, vec![green]);
        }
        let tree = Tree::new(green);
        assert_eq!(tree.root().descendants().count(), 100_001);
        assert_eq!(tree.root().tokens().count(), 1);
        assert_eq!(tree, tree.clone());
        assert_eq!(tree.to_syntax().tokens().count(), 1);
    }
}
