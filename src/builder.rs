//! The grammar-facing [`Builder`]: a token cursor and a tree builder in one.

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::Cell;
use core::fmt;

use syntax_lang::{Token, TokenKind};

use crate::green::Green;

/// The cursor and tree builder a [`Grammar`](crate::Grammar) parses with.
///
/// A grammar never creates a `Builder`; the [`Document`](crate::Document) hands
/// one to [`Grammar::parse`](crate::Grammar::parse), already positioned over
/// the tokens to parse. The grammar reads *significant* tokens through
/// [`peek`](Self::peek), [`nth`](Self::nth), [`at`](Self::at), and
/// [`at_end`](Self::at_end), consumes them with [`bump`](Self::bump) and
/// [`eat`](Self::eat), and groups them into nodes with [`start`](Self::start)
/// and [`finish`](Self::finish). A node can also be opened *retroactively*
/// around children already built, through [`checkpoint`](Self::checkpoint) and
/// [`start_at`](Self::start_at) — the move a precedence parser needs to wrap a
/// left operand once it sees the operator.
///
/// # Trivia is placed for you
///
/// The grammar never sees trivia (tokens whose kind reports
/// [`TokenKind::is_trivia`]) or end-of-input markers
/// ([`TokenKind::is_eof`]); the builder threads them into the tree itself.
/// Trivia is placed when the token after it is consumed: between two tokens of
/// a node it lands inside that node, and before a node's first token it lands
/// in the enclosing node, ahead of the node. So a node that holds any token
/// begins and ends on a significant one, and a node that holds none (a marker
/// for something missing, say) sits right after the last token before it.
/// That keeps node spans tight around their syntax, which is what lets a node
/// be relexed and reparsed on its own.
///
/// # The tree is always lossless
///
/// Every token ends up in the tree, whatever the grammar does: tokens left
/// over when the grammar returns are placed at the end, and nodes it leaves
/// open are closed. Misuse — finishing more nodes than were started, or a
/// stale checkpoint — is absorbed rather than panicking. A careless grammar
/// produces a lopsided tree, never a lossy one.
///
/// # Examples
///
/// A grammar for parenthesised lists such as `(a (b c))`. The `list` function
/// is the heart of it: open a node, consume the `(`, parse elements until the
/// `)`, close the node.
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
///         // One token per byte keeps the example short: `(`, `)`, space, atom.
///         for (i, byte) in text.bytes().enumerate() {
///             let kind = match byte {
///                 b'(' => K::Open,
///                 b')' => K::Close,
///                 b' ' => K::Space,
///                 _ => K::Atom,
///             };
///             tokens.push(Token::new(kind, Span::new(i as u32, i as u32 + 1)));
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
/// }
///
/// fn list(b: &mut Builder<'_, K>) {
///     b.start(K::List);
///     b.bump(); // `(`
///     while !b.at(K::Close) && !b.at_end() {
///         if b.at(K::Open) { list(b) } else { b.bump() }
///     }
///     b.eat(K::Close);
///     b.finish();
/// }
///
/// let doc = Document::new(Lists, "(a (b c))")?;
/// let lists: Vec<Span> = doc
///     .tree()
///     .root()
///     .descendants()
///     .filter(|n| n.kind() == K::List)
///     .map(|n| n.span())
///     .collect();
/// assert_eq!(lists, [Span::new(0, 9), Span::new(3, 8)]);
/// # Ok::<(), incremental_lang::Error>(())
/// ```
pub struct Builder<'a, K> {
    tokens: &'a [Token<K>],
    /// Index of the first token not yet placed in the tree.
    placed: usize,
    /// Index of the next significant token at or after `placed`
    /// (`tokens.len()` once none remain).
    next: usize,
    /// Children built so far, for every open node at once: each open node owns
    /// the suffix starting at its `first` index. One flat, pooled buffer
    /// instead of a vector per node.
    children: &'a mut Vec<Green<K>>,
    open: &'a mut Vec<Open<K>>,
    /// Count of significant tokens consumed so far. A node opened since the
    /// last one was consumed is *fresh*: pending trivia goes in front of it.
    epoch: usize,
    /// Set the first time the grammar looks past the last token. A reparse of a
    /// single node is accepted only if this stays clear: a grammar that never
    /// saw the end of the node's tokens made every decision from those tokens
    /// alone, so it would have made the same decisions with the rest of the
    /// document after them.
    saw_end: Cell<bool>,
}

/// A node that has been started and not yet finished.
pub(crate) struct Open<K> {
    kind: K,
    /// Index in the shared children buffer where this node's children begin.
    first: usize,
    /// The builder's `epoch` when the node was opened; [`RETROACTIVE`] for a
    /// node opened by `start_at` around tokens already consumed, which is
    /// never fresh.
    epoch: usize,
}

/// The `epoch` of a node opened around children that already exist.
const RETROACTIVE: usize = usize::MAX;

/// A position among the children of the node being built, recorded by
/// [`Builder::checkpoint`] so that [`Builder::start_at`] can later open a node
/// that begins there.
///
/// # Examples
///
/// Wrapping `a + b` in a `Sum` node only after the `+` has been seen:
///
/// ```
/// use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
///
/// #[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// enum K { Root, Sum, Num, Plus }
/// impl TokenKind for K {}
///
/// struct Sums;
/// impl Grammar for Sums {
///     type Kind = K;
///     const ROOT: K = K::Root;
///     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
///         for (i, byte) in text.bytes().enumerate() {
///             let kind = if byte == b'+' { K::Plus } else { K::Num };
///             tokens.push(Token::new(kind, Span::new(i as u32, i as u32 + 1)));
///         }
///     }
///     fn parse(&self, _: K, b: &mut Builder<'_, K>) {
///         b.start(K::Root);
///         let lhs = b.checkpoint();
///         b.bump(); // the first number
///         while b.eat(K::Plus) {
///             b.bump(); // the next number
///             b.start_at(lhs, K::Sum); // wrap everything since `lhs`
///             b.finish();
///         }
///         b.finish();
///     }
/// }
///
/// // Left-associative: ((1+2)+3)
/// let doc = Document::new(Sums, "1+2+3")?;
/// let sums: Vec<Span> = doc
///     .tree()
///     .root()
///     .descendants()
///     .filter(|n| n.kind() == K::Sum)
///     .map(|n| n.span())
///     .collect();
/// assert_eq!(sums, [Span::new(0, 5), Span::new(0, 3)]);
/// # Ok::<(), incremental_lang::Error>(())
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Checkpoint {
    /// Index in the children buffer.
    index: usize,
    /// The builder's epoch when the checkpoint was taken: if no token has
    /// been consumed since, a node opened here is still fresh.
    epoch: usize,
}

impl<'a, K: TokenKind + Copy + Eq> Builder<'a, K> {
    /// A builder over `tokens`, building into the pooled `children` and `open`
    /// buffers (cleared here).
    pub(crate) fn new(
        tokens: &'a [Token<K>],
        children: &'a mut Vec<Green<K>>,
        open: &'a mut Vec<Open<K>>,
    ) -> Self {
        children.clear();
        open.clear();
        let mut builder = Self {
            tokens,
            placed: 0,
            next: 0,
            children,
            open,
            epoch: 0,
            saw_end: Cell::new(false),
        };
        builder.next = builder.significant_from(0);
        builder
    }

    /// The kind of the next significant token, or `None` at the end of input.
    ///
    /// Trivia and end-of-input markers are skipped; the grammar never sees
    /// them.
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
    ///         // Leading spaces are trivia: the first thing peek sees is a word.
    ///         assert_eq!(b.peek(), Some(K::Word));
    ///         while b.peek().is_some() {
    ///             b.bump();
    ///         }
    ///         b.finish();
    ///     }
    /// }
    ///
    /// let doc = Document::new(Words, "   ab")?;
    /// assert_eq!(doc.tree().root().tokens().count(), 5);
    /// # Ok::<(), incremental_lang::Error>(())
    /// ```
    #[inline]
    #[must_use]
    pub fn peek(&self) -> Option<K> {
        match self.tokens.get(self.next) {
            Some(token) => Some(token.kind),
            None => {
                self.saw_end.set(true);
                None
            }
        }
    }

    /// The kind of the significant token `n` places ahead (`nth(0)` is
    /// [`peek`](Self::peek)), or `None` if the input ends first.
    ///
    /// Use it for decisions that need more than one token of lookahead. It
    /// scans forward over trivia, so its cost grows with `n`.
    ///
    /// # Examples
    ///
    /// Telling a key-value pair `k = v` from a bare value `v` by looking two
    /// tokens ahead:
    ///
    /// ```
    /// use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
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
    ///         while !b.at_end() {
    ///             if b.nth(1) == Some(K::Eq) {
    ///                 b.start(K::Pair);
    ///                 b.bump(); // key
    ///                 b.bump(); // `=`
    ///                 b.bump(); // value
    ///                 b.finish();
    ///             } else {
    ///                 b.bump();
    ///             }
    ///         }
    ///         b.finish();
    ///     }
    /// }
    ///
    /// let doc = Document::new(Pairs, "xk=v")?;
    /// let pair = doc.tree().root().descendants().find(|n| n.kind() == K::Pair);
    /// assert_eq!(pair.map(|n| n.span()), Some(Span::new(1, 4)));
    /// # Ok::<(), incremental_lang::Error>(())
    /// ```
    #[must_use]
    pub fn nth(&self, n: usize) -> Option<K> {
        let mut index = self.next;
        for _ in 0..n {
            if index >= self.tokens.len() {
                break;
            }
            index = self.significant_from(index + 1);
        }
        match self.tokens.get(index) {
            Some(token) => Some(token.kind),
            None => {
                self.saw_end.set(true);
                None
            }
        }
    }

    /// Whether the next significant token has kind `kind`.
    ///
    /// # Examples
    ///
    /// ```
    /// use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
    ///
    /// #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    /// enum K { Root, Bang, Word }
    /// impl TokenKind for K {}
    ///
    /// struct Bangs;
    /// impl Grammar for Bangs {
    ///     type Kind = K;
    ///     const ROOT: K = K::Root;
    ///     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
    ///         for (i, byte) in text.bytes().enumerate() {
    ///             let kind = if byte == b'!' { K::Bang } else { K::Word };
    ///             tokens.push(Token::new(kind, Span::new(i as u32, i as u32 + 1)));
    ///         }
    ///     }
    ///     fn parse(&self, _: K, b: &mut Builder<'_, K>) {
    ///         b.start(K::Root);
    ///         assert!(b.at(K::Bang));
    ///         assert!(!b.at(K::Word));
    ///         while !b.at_end() {
    ///             b.bump();
    ///         }
    ///         b.finish();
    ///     }
    /// }
    ///
    /// let doc = Document::new(Bangs, "!x")?;
    /// assert_eq!(doc.tree().root().kind(), K::Root);
    /// # Ok::<(), incremental_lang::Error>(())
    /// ```
    #[inline]
    #[must_use]
    pub fn at(&self, kind: K) -> bool {
        self.peek() == Some(kind)
    }

    /// Whether every significant token has been consumed.
    ///
    /// Loops over a node's contents should test for their closing token
    /// *before* testing for the end: `while !b.at(Close) && !b.at_end()`. When
    /// a single node is reparsed on its own, any look past its last token marks
    /// the attempt as unsafe and the document falls back to a larger reparse;
    /// testing for the closer first means a well-formed node never looks.
    ///
    /// # Examples
    ///
    /// ```
    /// use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
    ///
    /// #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    /// enum K { Root, Word }
    /// impl TokenKind for K {}
    ///
    /// struct Count;
    /// impl Grammar for Count {
    ///     type Kind = K;
    ///     const ROOT: K = K::Root;
    ///     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
    ///         for i in 0..text.len() as u32 {
    ///             tokens.push(Token::new(K::Word, Span::new(i, i + 1)));
    ///         }
    ///     }
    ///     fn parse(&self, _: K, b: &mut Builder<'_, K>) {
    ///         b.start(K::Root);
    ///         while !b.at_end() {
    ///             b.bump();
    ///         }
    ///         b.finish();
    ///     }
    /// }
    ///
    /// let doc = Document::new(Count, "abc")?;
    /// assert_eq!(doc.tree().root().children().len(), 3);
    /// # Ok::<(), incremental_lang::Error>(())
    /// ```
    #[inline]
    #[must_use]
    pub fn at_end(&self) -> bool {
        self.peek().is_none()
    }

    /// Consumes the next significant token into the node being built, along
    /// with any trivia before it. At the end of input it does nothing.
    ///
    /// # Examples
    ///
    /// ```
    /// use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
    ///
    /// #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    /// enum K { Root, Item, Word, Space }
    /// impl TokenKind for K {
    ///     fn is_trivia(&self) -> bool { matches!(self, K::Space) }
    /// }
    ///
    /// struct Items;
    /// impl Grammar for Items {
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
    ///         b.start(K::Item);
    ///         b.bump();
    ///         b.bump(); // takes the space before it along
    ///         b.finish();
    ///         b.finish();
    ///     }
    /// }
    ///
    /// let doc = Document::new(Items, "a b")?;
    /// let item = doc.tree().root().descendants().find(|n| n.kind() == K::Item);
    /// assert_eq!(item.map(|n| n.span()), Some(Span::new(0, 3)));
    /// # Ok::<(), incremental_lang::Error>(())
    /// ```
    #[inline]
    pub fn bump(&mut self) {
        let Some(token) = self.tokens.get(self.next) else {
            self.saw_end.set(true);
            return;
        };
        self.place_trivia();
        self.children.push(leaf(token));
        self.placed = self.next + 1;
        self.next = self.significant_from(self.placed);
        self.epoch += 1;
    }

    /// Consumes the next significant token if it has kind `kind`, reporting
    /// whether it did.
    ///
    /// # Examples
    ///
    /// ```
    /// use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
    ///
    /// #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    /// enum K { Root, Word, Semi }
    /// impl TokenKind for K {}
    ///
    /// struct Stmts;
    /// impl Grammar for Stmts {
    ///     type Kind = K;
    ///     const ROOT: K = K::Root;
    ///     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
    ///         for (i, byte) in text.bytes().enumerate() {
    ///             let kind = if byte == b';' { K::Semi } else { K::Word };
    ///             tokens.push(Token::new(kind, Span::new(i as u32, i as u32 + 1)));
    ///         }
    ///     }
    ///     fn parse(&self, _: K, b: &mut Builder<'_, K>) {
    ///         b.start(K::Root);
    ///         b.bump();
    ///         assert!(b.eat(K::Semi));  // present: consumed
    ///         assert!(!b.eat(K::Semi)); // absent: nothing consumed
    ///         b.finish();
    ///     }
    /// }
    ///
    /// let doc = Document::new(Stmts, "x;")?;
    /// assert_eq!(doc.tree().root().tokens().count(), 2);
    /// # Ok::<(), incremental_lang::Error>(())
    /// ```
    #[inline]
    pub fn eat(&mut self, kind: K) -> bool {
        if self.at(kind) {
            self.bump();
            true
        } else {
            false
        }
    }

    /// Opens a node of kind `kind`. Everything consumed until the matching
    /// [`finish`](Self::finish) becomes its children.
    ///
    /// Trivia between the previous token and the node's first token is placed
    /// in the *enclosing* node, ahead of this one, so the node starts on its
    /// first real token.
    ///
    /// # Examples
    ///
    /// ```
    /// use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
    ///
    /// #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    /// enum K { Root, Word, Letter, Space }
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
    ///             let kind = if byte == b' ' { K::Space } else { K::Letter };
    ///             tokens.push(Token::new(kind, Span::new(i as u32, i as u32 + 1)));
    ///         }
    ///     }
    ///     fn parse(&self, _: K, b: &mut Builder<'_, K>) {
    ///         b.start(K::Root);
    ///         while !b.at_end() {
    ///             b.start(K::Word);
    ///             b.bump();
    ///             b.finish();
    ///         }
    ///         b.finish();
    ///     }
    /// }
    ///
    /// // The spaces stay in Root; each Word covers only its letter.
    /// let doc = Document::new(Words, " a  b")?;
    /// let words: Vec<Span> = doc
    ///     .tree()
    ///     .root()
    ///     .descendants()
    ///     .filter(|n| n.kind() == K::Word)
    ///     .map(|n| n.span())
    ///     .collect();
    /// assert_eq!(words, [Span::new(1, 2), Span::new(4, 5)]);
    /// # Ok::<(), incremental_lang::Error>(())
    /// ```
    pub fn start(&mut self, kind: K) {
        self.open.push(Open {
            kind,
            first: self.children.len(),
            epoch: self.epoch,
        });
    }

    /// Closes the most recently opened node. With no node open it does
    /// nothing.
    ///
    /// Trivia after the node's last token is not taken along: it is placed
    /// later, in whichever node is open when the next token is consumed.
    ///
    /// # Examples
    ///
    /// ```
    /// use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
    ///
    /// #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    /// enum K { Root, Group, Word, Space }
    /// impl TokenKind for K {
    ///     fn is_trivia(&self) -> bool { matches!(self, K::Space) }
    /// }
    ///
    /// struct Group;
    /// impl Grammar for Group {
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
    ///         b.start(K::Group);
    ///         b.bump();
    ///         b.finish(); // the space after `a` is not part of Group
    ///         b.bump();
    ///         b.finish();
    ///     }
    /// }
    ///
    /// let doc = Document::new(Group, "a b")?;
    /// let group = doc.tree().root().descendants().find(|n| n.kind() == K::Group);
    /// assert_eq!(group.map(|n| n.span()), Some(Span::new(0, 1)));
    /// # Ok::<(), incremental_lang::Error>(())
    /// ```
    pub fn finish(&mut self) {
        let Some(open) = self.open.pop() else {
            return;
        };
        let first = open.first.min(self.children.len());
        let children: Arc<[Green<K>]> = self.children.drain(first..).collect();
        self.children.push(Green::node(open.kind, children));
    }

    /// Records the current position among the children of the node being
    /// built, for a later [`start_at`](Self::start_at).
    ///
    /// A node opened at the checkpoint never begins with trivia: trivia that
    /// ends up at the checkpoint stays in the enclosing node.
    ///
    /// # Examples
    ///
    /// See [`Checkpoint`] for a left-associative operator chain.
    ///
    /// ```
    /// use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
    ///
    /// #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    /// enum K { Root, Call, Name, Args }
    /// impl TokenKind for K {}
    ///
    /// // `f` alone is a name; `f()` becomes a Call wrapping the name — decided
    /// // only after the name was consumed.
    /// struct Calls;
    /// impl Grammar for Calls {
    ///     type Kind = K;
    ///     const ROOT: K = K::Root;
    ///     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
    ///         let end = text.len() as u32;
    ///         match text.find('(') {
    ///             Some(i) => {
    ///                 tokens.push(Token::new(K::Name, Span::new(0, i as u32)));
    ///                 tokens.push(Token::new(K::Args, Span::new(i as u32, end)));
    ///             }
    ///             None => tokens.push(Token::new(K::Name, Span::new(0, end))),
    ///         }
    ///     }
    ///     fn parse(&self, _: K, b: &mut Builder<'_, K>) {
    ///         b.start(K::Root);
    ///         let callee = b.checkpoint();
    ///         b.bump();
    ///         if b.at(K::Args) {
    ///             b.start_at(callee, K::Call);
    ///             b.bump();
    ///             b.finish();
    ///         }
    ///         b.finish();
    ///     }
    /// }
    ///
    /// let call = Document::new(Calls, "f()")?;
    /// assert!(call.tree().root().descendants().any(|n| n.kind() == K::Call));
    ///
    /// let name = Document::new(Calls, "f")?;
    /// assert!(!name.tree().root().descendants().any(|n| n.kind() == K::Call));
    /// # Ok::<(), incremental_lang::Error>(())
    /// ```
    #[must_use]
    pub fn checkpoint(&mut self) -> Checkpoint {
        // Trivia waiting in front of a node opened since the last token goes
        // ahead of that node when the next token is consumed, shifting every
        // index after it. Placing it now — where it would go anyway — keeps
        // the index recorded here valid.
        let epoch = self.epoch;
        if self.open.last().is_some_and(|open| open.epoch == epoch) {
            self.place_trivia();
        }
        Checkpoint {
            index: self.children.len(),
            epoch,
        }
    }

    /// Opens a node of kind `kind` that begins at `checkpoint`, adopting every
    /// child built since then. Close it with [`finish`](Self::finish) as usual.
    ///
    /// The checkpoint must come from the node currently being built and from
    /// before any node that is still open; one that does not is clamped into
    /// range rather than panicking.
    ///
    /// # Examples
    ///
    /// See [`Checkpoint`] and [`checkpoint`](Self::checkpoint).
    ///
    /// ```
    /// use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
    ///
    /// #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    /// enum K { Root, Pair, Word }
    /// impl TokenKind for K {}
    ///
    /// struct Pairs;
    /// impl Grammar for Pairs {
    ///     type Kind = K;
    ///     const ROOT: K = K::Root;
    ///     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
    ///         for i in 0..text.len() as u32 {
    ///             tokens.push(Token::new(K::Word, Span::new(i, i + 1)));
    ///         }
    ///     }
    ///     fn parse(&self, _: K, b: &mut Builder<'_, K>) {
    ///         b.start(K::Root);
    ///         while !b.at_end() {
    ///             let first = b.checkpoint();
    ///             b.bump();
    ///             b.bump();
    ///             b.start_at(first, K::Pair); // group the two words just consumed
    ///             b.finish();
    ///         }
    ///         b.finish();
    ///     }
    /// }
    ///
    /// let doc = Document::new(Pairs, "abcd")?;
    /// let pairs: Vec<Span> = doc
    ///     .tree()
    ///     .root()
    ///     .children()
    ///     .map(|child| child.span())
    ///     .collect();
    /// assert_eq!(pairs, [Span::new(0, 2), Span::new(2, 4)]);
    /// # Ok::<(), incremental_lang::Error>(())
    /// ```
    pub fn start_at(&mut self, checkpoint: Checkpoint, kind: K) {
        let floor = self.open.last().map_or(0, |open| open.first);
        let mut first = checkpoint.index.max(floor).min(self.children.len());
        while self
            .children
            .get(first)
            .is_some_and(|child| !child.is_node() && !is_significant(&child.kind))
        {
            first += 1;
        }
        self.open.push(Open {
            kind,
            first,
            // With no token consumed since the checkpoint, the node has adopted
            // at most empty nodes; it is fresh, so trivia still goes ahead of
            // it. Otherwise it wraps tokens already placed.
            epoch: if checkpoint.epoch == self.epoch {
                self.epoch
            } else {
                RETROACTIVE
            },
        });
    }

    /// Finishes a whole-document parse: closes what the grammar left open,
    /// places leftover tokens, and returns the root node of kind `root`.
    ///
    /// When the grammar built exactly one top-level node of kind `root`, any
    /// tokens around it (leading trivia, trailing trivia, leftovers) are moved
    /// inside it; otherwise everything is wrapped in a new `root` node.
    pub(crate) fn finish_root(mut self, root: K) -> Green<K> {
        self.close_all();
        let mut nodes = self
            .children
            .iter()
            .enumerate()
            .filter(|(_, child)| child.is_node());
        let single = match (nodes.next(), nodes.next()) {
            (Some((index, child)), None) if child.kind == root => Some(index),
            _ => None,
        };
        let tree = match single {
            Some(_) if self.children.len() == 1 => self.children.pop(),
            Some(index) => {
                let (before, rest) = self.children.split_at(index);
                let (built, after) = (&rest[0], &rest[1..]);
                let merged: Arc<[Green<K>]> = before
                    .iter()
                    .chain(built.children())
                    .chain(after)
                    .cloned()
                    .collect();
                Some(Green::node(root, merged))
            }
            None => None,
        };
        let tree = tree.unwrap_or_else(|| Green::node(root, self.children.drain(..).collect()));
        self.children.clear();
        tree
    }

    /// Finishes a single-node reparse, returning the node only if the attempt
    /// is safe to splice in: the grammar never looked past the last token, and
    /// it built exactly one node, of kind `kind`, holding every token.
    pub(crate) fn finish_reparse(mut self, kind: K) -> Option<Green<K>> {
        let saw_end = self.saw_end.get();
        self.close_all();
        let accept = !saw_end
            && matches!(self.children.as_slice(), [only] if only.is_node() && only.kind == kind);
        let node = if accept { self.children.pop() } else { None };
        self.children.clear();
        node
    }

    /// Closes every open node and places every remaining token at the end.
    fn close_all(&mut self) {
        while !self.open.is_empty() {
            self.finish();
        }
        let rest = self.tokens.get(self.placed..).unwrap_or_default();
        self.children.extend(rest.iter().map(leaf));
        self.placed = self.tokens.len();
    }

    /// Places the trivia waiting in front of the next significant token: ahead
    /// of every fresh node (opened since the last token was consumed), or at
    /// the end of the current node if there is none.
    fn place_trivia(&mut self) {
        let trivia = self.tokens.get(self.placed..self.next).unwrap_or_default();
        if trivia.is_empty() {
            return;
        }
        let epoch = self.epoch;
        let fresh = self
            .open
            .iter()
            .rev()
            .take_while(|open| open.epoch == epoch)
            .count();
        let anchor = match fresh {
            0 => self.children.len(),
            _ => self.open[self.open.len() - fresh].first,
        };
        // Fresh nodes hold no tokens yet — at most empty nodes — so only that
        // short tail of the buffer shifts.
        drop(
            self.children
                .splice(anchor..anchor, trivia.iter().map(leaf)),
        );
        for open in self.open.iter_mut().rev().take(fresh) {
            open.first += trivia.len();
        }
        self.placed = self.next;
    }

    /// Index of the first significant token at or after `from`.
    #[inline]
    fn significant_from(&self, from: usize) -> usize {
        let rest = self.tokens.get(from..).unwrap_or_default();
        rest.iter()
            .position(|token| is_significant(&token.kind))
            .map_or(self.tokens.len(), |offset| from + offset)
    }
}

/// Whether the grammar sees tokens of this kind: neither trivia nor an
/// end-of-input marker.
#[inline]
fn is_significant<K: TokenKind>(kind: &K) -> bool {
    !kind.is_trivia() && !kind.is_eof()
}

/// The green leaf for a token.
#[inline]
fn leaf<K: Copy>(token: &Token<K>) -> Green<K> {
    Green::token(token.kind, token.span.len())
}

impl<K: fmt::Debug> fmt::Debug for Builder<'_, K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Builder")
            .field("next", &self.tokens.get(self.next).map(|token| &token.kind))
            .field(
                "remaining",
                &(self.tokens.len() - self.placed.min(self.tokens.len())),
            )
            .field("open", &self.open.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syntax_lang::Span;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum K {
        Root,
        Node,
        Word,
        Space,
        Eof,
    }

    impl TokenKind for K {
        fn is_trivia(&self) -> bool {
            matches!(self, K::Space)
        }
        fn is_eof(&self) -> bool {
            matches!(self, K::Eof)
        }
    }

    /// Tokens of width one from a pattern: `w` word, `_` space.
    fn tokens(pattern: &str) -> Vec<Token<K>> {
        pattern
            .bytes()
            .enumerate()
            .map(|(i, byte)| {
                let kind = if byte == b'_' { K::Space } else { K::Word };
                let at = u32::try_from(i).unwrap_or(u32::MAX);
                Token::new(kind, Span::new(at, at + 1))
            })
            .collect()
    }

    fn kinds(green: &Green<K>) -> Vec<(K, u32, bool)> {
        green
            .children()
            .iter()
            .map(|child| (child.kind, child.width, child.is_node()))
            .collect()
    }

    fn with_builder<R>(tokens: &[Token<K>], run: impl FnOnce(Builder<'_, K>) -> R) -> R {
        let mut children = Vec::new();
        let mut open = Vec::new();
        run(Builder::new(tokens, &mut children, &mut open))
    }

    #[test]
    fn test_peek_skips_trivia_and_eof() {
        let mut toks = tokens("__w");
        toks.push(Token::new(K::Eof, Span::empty(3)));
        with_builder(&toks, |mut b| {
            assert_eq!(b.peek(), Some(K::Word));
            b.bump();
            assert_eq!(b.peek(), None);
            assert!(b.at_end());
        });
    }

    #[test]
    fn test_nth_counts_significant_tokens_only() {
        let toks = tokens("w_w__w");
        with_builder(&toks, |b| {
            assert_eq!(b.nth(0), Some(K::Word));
            assert_eq!(b.nth(2), Some(K::Word));
            assert_eq!(b.nth(3), None);
        });
    }

    #[test]
    fn test_start_places_leading_trivia_in_parent() {
        let toks = tokens("_w_");
        let root = with_builder(&toks, |mut b| {
            b.start(K::Root);
            b.start(K::Node);
            b.bump();
            b.finish();
            b.finish();
            b.finish_root(K::Root)
        });
        assert_eq!(
            kinds(&root),
            [
                (K::Space, 1, false),
                (K::Node, 1, true),
                (K::Space, 1, false)
            ]
        );
        assert_eq!(root.width, 3);
    }

    #[test]
    fn test_empty_node_sits_before_pending_trivia() {
        // Node holds `w` and an empty marker; the space after it stays outside.
        let toks = tokens("w_w");
        let root = with_builder(&toks, |mut b| {
            b.start(K::Root);
            b.start(K::Node);
            b.bump();
            b.start(K::Node);
            b.finish();
            b.finish();
            b.bump();
            b.finish();
            b.finish_root(K::Root)
        });
        assert_eq!(
            kinds(&root),
            [
                (K::Node, 1, true),
                (K::Space, 1, false),
                (K::Word, 1, false)
            ]
        );
        assert_eq!(kinds(&root.children()[0])[1], (K::Node, 0, true));
    }

    #[test]
    fn test_trivia_goes_ahead_of_every_fresh_node() {
        let toks = tokens("w__w");
        let root = with_builder(&toks, |mut b| {
            b.start(K::Root);
            b.bump();
            b.start(K::Node);
            b.start(K::Node);
            b.bump();
            b.finish();
            b.finish();
            b.finish();
            b.finish_root(K::Root)
        });
        assert_eq!(
            kinds(&root),
            [
                (K::Word, 1, false),
                (K::Space, 1, false),
                (K::Space, 1, false),
                (K::Node, 1, true)
            ]
        );
    }

    #[test]
    fn test_checkpoint_then_finish_leaves_no_trailing_trivia() {
        let toks = tokens("w_w");
        let root = with_builder(&toks, |mut b| {
            b.start(K::Root);
            b.start(K::Node);
            b.bump();
            let _unused = b.checkpoint();
            b.finish();
            b.bump();
            b.finish();
            b.finish_root(K::Root)
        });
        assert_eq!(
            kinds(&root),
            [
                (K::Node, 1, true),
                (K::Space, 1, false),
                (K::Word, 1, false)
            ]
        );
    }

    #[test]
    fn test_checkpoint_in_fresh_node_survives_pending_trivia() {
        // An empty marker, then a checkpoint, then a token: the node opened
        // at the checkpoint must wrap the token and not the marker.
        let toks = tokens("w_w");
        let root = with_builder(&toks, |mut b| {
            b.start(K::Root);
            b.bump();
            b.start(K::Node);
            b.start(K::Node);
            b.finish();
            let cp = b.checkpoint();
            b.bump();
            b.start_at(cp, K::Root);
            b.finish();
            b.finish();
            b.finish();
            b.finish_root(K::Root)
        });
        assert_eq!(
            kinds(&root),
            [
                (K::Word, 1, false),
                (K::Space, 1, false),
                (K::Node, 1, true)
            ]
        );
        let outer = &root.children()[2];
        assert_eq!(kinds(outer), [(K::Node, 0, true), (K::Root, 1, true)]);
    }

    #[test]
    fn test_start_at_with_nothing_adopted_keeps_trivia_outside() {
        // `a`, checkpoint, open a node there with nothing in it yet, then `b`:
        // the space before `b` belongs outside the node.
        let toks = tokens("w_w");
        let root = with_builder(&toks, |mut b| {
            b.start(K::Root);
            b.bump();
            let cp = b.checkpoint();
            b.start_at(cp, K::Node);
            b.bump();
            b.finish();
            b.finish();
            b.finish_root(K::Root)
        });
        assert_eq!(
            kinds(&root),
            [
                (K::Word, 1, false),
                (K::Space, 1, false),
                (K::Node, 1, true)
            ]
        );
    }

    #[test]
    fn test_finish_root_wraps_multiple_top_level_nodes() {
        let toks = tokens("ww");
        let root = with_builder(&toks, |mut b| {
            for _ in 0..2 {
                b.start(K::Node);
                b.bump();
                b.finish();
            }
            b.finish_root(K::Root)
        });
        assert_eq!(root.kind, K::Root);
        assert_eq!(kinds(&root), [(K::Node, 1, true), (K::Node, 1, true)]);
    }

    #[test]
    fn test_finish_root_keeps_leftovers_when_grammar_does_nothing() {
        let toks = tokens("w_w");
        let root = with_builder(&toks, |b| b.finish_root(K::Root));
        assert_eq!(root.kind, K::Root);
        assert_eq!(root.children().len(), 3);
        assert_eq!(root.width, 3);
    }

    #[test]
    fn test_finish_root_closes_open_nodes_then_places_leftovers() {
        let toks = tokens("ww");
        let root = with_builder(&toks, |mut b| {
            b.start(K::Root);
            b.start(K::Node);
            b.bump();
            b.finish_root(K::Root)
        });
        assert_eq!(kinds(&root), [(K::Node, 1, true), (K::Word, 1, false)]);
        assert_eq!(root.width, 2);
    }

    #[test]
    fn test_finish_without_open_node_is_ignored() {
        let toks = tokens("w");
        let root = with_builder(&toks, |mut b| {
            b.finish();
            b.start(K::Root);
            b.bump();
            b.finish();
            b.finish();
            b.finish_root(K::Root)
        });
        assert_eq!(kinds(&root), [(K::Word, 1, false)]);
    }

    #[test]
    fn test_start_at_wraps_children_since_checkpoint() {
        let toks = tokens("w_ww");
        let root = with_builder(&toks, |mut b| {
            b.start(K::Root);
            b.bump();
            let cp = b.checkpoint();
            b.bump();
            b.bump();
            b.start_at(cp, K::Node);
            b.finish();
            b.finish();
            b.finish_root(K::Root)
        });
        assert_eq!(
            kinds(&root),
            [
                (K::Word, 1, false),
                (K::Space, 1, false),
                (K::Node, 2, true)
            ]
        );
    }

    #[test]
    fn test_start_at_clamps_stale_checkpoint() {
        let toks = tokens("ww");
        let root = with_builder(&toks, |mut b| {
            b.start(K::Root);
            b.bump();
            b.start(K::Node);
            b.bump();
            // A checkpoint from outside the open Node is clamped to its start.
            b.start_at(
                Checkpoint {
                    index: 0,
                    epoch: RETROACTIVE,
                },
                K::Node,
            );
            b.finish();
            b.finish();
            b.finish();
            b.finish_root(K::Root)
        });
        assert_eq!(root.width, 2);
        assert_eq!(kinds(&root)[0], (K::Word, 1, false));
        assert_eq!(kinds(&root)[1], (K::Node, 1, true));
    }

    #[test]
    fn test_finish_reparse_accepts_single_node_of_kind() {
        let toks = tokens("w_w");
        let node = with_builder(&toks, |mut b| {
            b.start(K::Node);
            b.bump();
            b.bump();
            b.finish();
            b.finish_reparse(K::Node)
        });
        assert_eq!(node.map(|n| (n.kind, n.width)), Some((K::Node, 3)));
    }

    #[test]
    fn test_finish_reparse_rejects_when_end_was_seen() {
        let toks = tokens("w");
        let node = with_builder(&toks, |mut b| {
            b.start(K::Node);
            b.bump();
            let _ = b.at_end();
            b.finish();
            b.finish_reparse(K::Node)
        });
        assert!(node.is_none());
    }

    #[test]
    fn test_finish_reparse_rejects_leftovers_and_wrong_kind() {
        let toks = tokens("ww");
        let leftover = with_builder(&toks, |mut b| {
            b.start(K::Node);
            b.bump();
            b.finish();
            b.finish_reparse(K::Node)
        });
        assert!(leftover.is_none());

        let wrong = with_builder(&toks, |mut b| {
            b.start(K::Root);
            b.bump();
            b.bump();
            b.finish();
            b.finish_reparse(K::Node)
        });
        assert!(wrong.is_none());
    }

    #[test]
    fn test_bump_at_end_marks_end_seen() {
        let toks = tokens("");
        let node = with_builder(&toks, |mut b| {
            b.start(K::Node);
            b.bump();
            b.finish();
            b.finish_reparse(K::Node)
        });
        assert!(node.is_none());
    }

    #[test]
    fn test_eat_consumes_only_matching_kind() {
        let toks = tokens("w");
        with_builder(&toks, |mut b| {
            assert!(!b.eat(K::Space));
            assert!(b.eat(K::Word));
            assert!(!b.eat(K::Word));
        });
    }

    #[test]
    fn test_debug_reports_cursor() {
        let toks = tokens("w");
        let text = with_builder(&toks, |b| alloc::format!("{b:?}"));
        assert!(text.contains("Word"));
    }
}
