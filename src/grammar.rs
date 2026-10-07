//! The [`Grammar`] trait: what a language supplies so a document can be lexed,
//! parsed, and reparsed piece by piece.

use alloc::vec::Vec;

use syntax_lang::{Token, TokenKind};

use crate::builder::Builder;

/// A language: its token and node kinds, its lexer, and its parser.
///
/// A [`Document`](crate::Document) drives these three pieces. On creation it
/// lexes the whole text and parses it as one [`ROOT`](Self::ROOT) node. On each
/// edit it looks for the smallest node around the change whose kind is
/// [reparsable](Self::is_reparsable), relexes and reparses just that node, and
/// splices the result into the tree; when no such node can be reparsed safely
/// it falls back to a whole-document parse. A grammar that marks no kind
/// reparsable is still correct — every edit is then a full reparse.
///
/// # The kind type
///
/// One type names both node kinds and token kinds — the model `syntax-lang`
/// and `rowan` use. Typically it is a fieldless `enum` with composite variants
/// (`Object`, `Array`) and lexical ones (`LBrace`, `String`, `Whitespace`), and
/// its [`TokenKind`] implementation marks the trivia. Kinds are compared and
/// copied freely, so they must be `Copy + Eq`; keep them small.
///
/// # The lexer contract
///
/// [`lex`](Self::lex) must *tile* its input: the tokens it produces start at
/// byte 0, each begins where the previous one ended, and the last ends at the
/// end of the text. Zero-width tokens are allowed; zero-width end-of-input
/// markers ([`TokenKind::is_eof`]) are dropped. A lexer that leaves a gap or
/// overlap is rejected with [`Error::Tokens`](crate::Error::Tokens), never
/// trusted.
///
/// For a node to be relexed on its own, the lexer must also meet two
/// conditions:
///
/// - **Restartable.** Started at a token boundary, it produces the same tokens
///   a full lex produces from that point. A lexer whose output depends on text
///   far behind the current position (an indentation stack, a heredoc
///   terminator chosen earlier in the file) is not.
/// - **One character of lookahead.** Where a token ends may depend on at most
///   one character after it. A lexer that scans ahead and then backs off —
///   trying to match `"…"` up to a closing quote and, finding none, falling
///   back to a lone `"` token — looks arbitrarily far ahead: a quote typed
///   later, anywhere on the line, would change how text far before it lexes.
///   Let an unterminated literal run to the end of its line or of the input
///   instead, as most editor lexers do.
///
/// Hand-written lexers that consume a token while the next character fits it
/// meet both conditions. A lexer that cannot should mark no kind reparsable.
///
/// # The parser contract
///
/// [`parse`](Self::parse) builds one node of the requested kind from the
/// tokens the [`Builder`] offers. For a reparsable kind:
///
/// - **The node stands alone.** Parsing it makes the same decisions the
///   whole-document parse makes at that point; nothing about how it is parsed
///   depends on what surrounds it.
/// - **Nothing around it looks inside.** Outside the node, no decision may
///   depend on its contents beyond its first token: the decision to start it
///   rests on that token alone, no lookahead (`nth`) reaches past it into the
///   node, and nothing its parse computes — an element count, a flag — feeds
///   back into the parse around it.
///
/// # What the document checks
///
/// A single-node reparse is spliced in only if all of the following hold;
/// otherwise the document tries a larger node, then the whole document:
///
/// - the edit lies strictly inside the node, so its first and last bytes are
///   untouched;
/// - relexing the node's new text, together with the token on either side of
///   it, reproduces those two neighbours exactly — given the lexer contract, no
///   token can have grown across the node's boundary;
/// - the node still begins and ends with tokens of the same kinds as before;
/// - the grammar built exactly one node, of the same kind, holding every token,
///   and never looked past the node's last token.
///
/// That last check is what catches an edit that unbalances brackets: a list
/// that has lost its closing `)` keeps asking for more tokens, so it looks past
/// its end, and the reparse widens instead of producing a tree that a fresh
/// parse would disagree with.
///
/// These checks enforce the parts of the contract that can be checked cheaply.
/// The rest — lexer lookahead, and code outside a node depending on its
/// contents — cannot be observed from a single reparse, so a grammar that
/// breaks it can end up with a tree that differs from a fresh parse. Holding
/// a grammar's incremental trees to fresh parses in a property test, as this
/// crate's own tests do, is the way to be sure.
///
/// # Choosing reparsable kinds
///
/// Good candidates begin and end with tokens that never merge with their
/// neighbours — bracketed constructs such as blocks, objects, arrays, and
/// argument lists. Their contents are where nearly all typing happens, and
/// their extent is fixed by the brackets. Kinds without delimiters, or whose
/// meaning depends on context, should not be marked.
///
/// # Examples
///
/// A complete grammar for nested parenthesised lists, with `List` reparsable:
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
///     b.bump(); // `(`
///     while !b.at(K::Close) && !b.at_end() {
///         if b.at(K::Open) { list(b) } else { b.bump() }
///     }
///     b.eat(K::Close);
///     b.finish();
/// }
///
/// let mut doc = Document::new(Lists, "(define (sq x) (mul x x))")?;
///
/// // Rename `x` to `n` inside the innermost list: only `(mul x x)` is redone.
/// let edit = doc.edit(Span::new(20, 21), "n")?;
/// assert_eq!(edit.kind(), K::List);
/// assert_eq!(edit.span(), Span::new(15, 24));
/// assert_eq!(doc.text(), "(define (sq x) (mul n x))");
/// # Ok::<(), incremental_lang::Error>(())
/// ```
pub trait Grammar {
    /// The kind type shared by nodes and tokens.
    type Kind: TokenKind + Copy + Eq;

    /// The kind of the node that spans the whole document.
    ///
    /// [`parse`](Self::parse) is called with this kind for every whole-document
    /// parse. The document guarantees the tree's root has this kind even if the
    /// grammar builds something else at the top level.
    const ROOT: Self::Kind;

    /// Lexes `text`, appending its tokens to `tokens` (which arrives empty).
    ///
    /// Spans are byte offsets relative to the start of `text`, which may be
    /// the whole document or a slice of it around a node being relexed. The
    /// tokens must tile `text` exactly; see [the lexer contract](Self#the-lexer-contract).
    ///
    /// # Examples
    ///
    /// ```
    /// use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
    ///
    /// #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    /// enum K { Root, Digit, Other }
    /// impl TokenKind for K {}
    ///
    /// struct Digits;
    /// impl Grammar for Digits {
    ///     type Kind = K;
    ///     const ROOT: K = K::Root;
    ///     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
    ///         // char_indices keeps every token on a UTF-8 boundary.
    ///         for (i, c) in text.char_indices() {
    ///             let kind = if c.is_ascii_digit() { K::Digit } else { K::Other };
    ///             let end = i + c.len_utf8();
    ///             tokens.push(Token::new(kind, Span::new(i as u32, end as u32)));
    ///         }
    ///     }
    ///     fn parse(&self, _: K, b: &mut Builder<'_, K>) {
    ///         b.start(K::Root);
    ///         while !b.at_end() { b.bump(); }
    ///         b.finish();
    ///     }
    /// }
    ///
    /// let doc = Document::new(Digits, "a1é")?;
    /// let widths: Vec<u32> = doc.tree().root().tokens().map(|t| t.span.len()).collect();
    /// assert_eq!(widths, [1, 1, 2]);
    /// # Ok::<(), incremental_lang::Error>(())
    /// ```
    fn lex(&self, text: &str, tokens: &mut Vec<Token<Self::Kind>>);

    /// Parses one node of kind `kind` from the tokens `builder` offers.
    ///
    /// Called with [`ROOT`](Self::ROOT) to parse a whole document, and with a
    /// [reparsable](Self::is_reparsable) kind to reparse a single node. In the
    /// second case the builder offers exactly that node's tokens. A grammar
    /// usually dispatches on `kind` to the function that parses that construct
    /// — the same function its whole-document parse calls when it meets one.
    ///
    /// # Examples
    ///
    /// See [the trait-level example](Self#examples) for a grammar that
    /// dispatches on `kind`.
    ///
    /// ```
    /// use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};
    ///
    /// #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    /// enum K { Root, Char }
    /// impl TokenKind for K {}
    ///
    /// struct Flat;
    /// impl Grammar for Flat {
    ///     type Kind = K;
    ///     const ROOT: K = K::Root;
    ///     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
    ///         for (i, c) in text.char_indices() {
    ///             let end = (i + c.len_utf8()) as u32;
    ///             tokens.push(Token::new(K::Char, Span::new(i as u32, end)));
    ///         }
    ///     }
    ///     // Only ever called with ROOT: no kind is reparsable.
    ///     fn parse(&self, _: K, b: &mut Builder<'_, K>) {
    ///         b.start(K::Root);
    ///         while !b.at_end() { b.bump(); }
    ///         b.finish();
    ///     }
    /// }
    ///
    /// let doc = Document::new(Flat, "xyz")?;
    /// assert_eq!(doc.tree().root().children().len(), 3);
    /// # Ok::<(), incremental_lang::Error>(())
    /// ```
    fn parse(&self, kind: Self::Kind, builder: &mut Builder<'_, Self::Kind>);

    /// Whether a node of kind `kind` may be reparsed on its own.
    ///
    /// The default is `false` for every kind: always correct, never
    /// incremental. See [choosing reparsable kinds](Self#choosing-reparsable-kinds).
    ///
    /// # Examples
    ///
    /// ```
    /// use incremental_lang::{Builder, Grammar, Span, Token, TokenKind};
    ///
    /// #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    /// enum K { Root, Block, Statement }
    /// impl TokenKind for K {}
    ///
    /// struct Blocks;
    /// impl Grammar for Blocks {
    ///     type Kind = K;
    ///     const ROOT: K = K::Root;
    ///     fn lex(&self, _: &str, _: &mut Vec<Token<K>>) {}
    ///     fn parse(&self, _: K, _: &mut Builder<'_, K>) {}
    ///     fn is_reparsable(&self, kind: K) -> bool {
    ///         // `{ ... }` blocks are delimited; statements are not.
    ///         kind == K::Block
    ///     }
    /// }
    ///
    /// assert!(Blocks.is_reparsable(K::Block));
    /// assert!(!Blocks.is_reparsable(K::Statement));
    /// ```
    fn is_reparsable(&self, kind: Self::Kind) -> bool {
        let _ = kind;
        false
    }
}
