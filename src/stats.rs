//! [`Stats`] and [`Reparse`]: what the document did, per edit and in total.

use syntax_lang::Span;

/// What one call to [`Document::edit`](crate::Document::edit) rebuilt.
///
/// The [`span`](Self::span) is the region of the *new* text whose syntax was
/// rebuilt; everything outside it kept its tree, shifted by the edit. An
/// editor can limit re-highlighting, re-folding, or re-checking to that
/// region.
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
/// let mut doc = Document::new(Lists, "(a) (b c)")?;
///
/// // Typing inside `(b c)` rebuilds just that list.
/// let local = doc.edit(Span::new(6, 6), "x")?;
/// assert_eq!((local.kind(), local.span()), (K::List, Span::new(4, 10)));
/// assert!(!local.is_full());
///
/// // Deleting a `(` unbalances the text: the whole document is reparsed.
/// let global = doc.edit(Span::new(0, 1), "")?;
/// assert!(global.is_full());
/// assert_eq!(global.span(), Span::new(0, 9));
/// # Ok::<(), incremental_lang::Error>(())
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reparse<K> {
    kind: K,
    span: Span,
    full: bool,
}

impl<K: Copy> Reparse<K> {
    pub(crate) const fn new(kind: K, span: Span, full: bool) -> Self {
        Self { kind, span, full }
    }

    /// The kind of the node that was rebuilt: a reparsable kind, or the
    /// grammar's [`ROOT`](crate::Grammar::ROOT) after a full reparse.
    ///
    /// # Examples
    ///
    /// See [`Reparse`].
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
    /// // A grammar with no reparsable kinds always rebuilds the root.
    /// let mut doc = Document::new(Chars, "ab")?;
    /// assert_eq!(doc.edit(Span::new(1, 1), "-")?.kind(), K::Root);
    /// # Ok::<(), incremental_lang::Error>(())
    /// ```
    #[inline]
    #[must_use]
    pub const fn kind(&self) -> K {
        self.kind
    }

    /// The region of the new text whose syntax was rebuilt.
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
    /// // A full reparse covers the whole new text.
    /// assert_eq!(doc.edit(Span::new(2, 2), "cd")?.span(), Span::new(0, 4));
    /// # Ok::<(), incremental_lang::Error>(())
    /// ```
    #[inline]
    #[must_use]
    pub const fn span(&self) -> Span {
        self.span
    }

    /// Whether the whole document was reparsed, rather than a single node.
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
    /// assert!(doc.edit(Span::new(0, 1), "z")?.is_full());
    /// # Ok::<(), incremental_lang::Error>(())
    /// ```
    #[inline]
    #[must_use]
    pub const fn is_full(&self) -> bool {
        self.full
    }
}

/// Running counters a [`Document`](crate::Document) keeps about its own work.
///
/// They answer the operational questions: are edits being served
/// incrementally (`partial` high, `full` low), are the grammar's reparsable
/// kinds being rejected (`rejected` climbing), and how much text is being
/// relexed per edit (`bytes` against `edits`). All counters start at zero and
/// only grow; read them with [`Document::stats`](crate::Document::stats).
///
/// # Examples
///
/// ```
/// # use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
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
/// let mut doc = Document::new(Lists, "(a b) (c d)")?;
/// doc.edit(Span::new(2, 3), "x")?; // inside the first list
/// doc.edit(Span::new(8, 9), "y")?; // inside the second
///
/// let stats = doc.stats();
/// assert_eq!(stats.edits, 2);
/// assert_eq!(stats.partial, 2);
/// assert_eq!(stats.full, 1); // the initial parse
/// assert_eq!(stats.rejected, 0);
/// # Ok::<(), incremental_lang::Error>(())
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct Stats {
    /// Edits applied. Refused edits are not counted.
    pub edits: u64,
    /// Edits served by reparsing a single node rather than the whole document.
    pub partial: u64,
    /// Whole-document parses, including the one that created the document.
    pub full: u64,
    /// Single-node reparse attempts that failed a safety check and gave way to
    /// a larger node or a full reparse. A steadily rising count means the
    /// grammar's reparsable kinds rarely survive real edits.
    pub rejected: u64,
    /// Total bytes of text handed to the lexer, across every parse and
    /// reparse attempt — a direct measure of the work done.
    pub bytes: u64,
}
