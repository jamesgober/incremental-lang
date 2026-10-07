//! [`Error`]: why a document could not be created or an edit was refused.

use core::fmt;

use syntax_lang::Span;

/// Why [`Document::new`](crate::Document::new) or
/// [`Document::edit`](crate::Document::edit) failed.
///
/// A failed edit changes nothing: the document's text, tree, and statistics
/// are exactly as they were before the call. Every variant carries the byte
/// offsets needed to report or correct the problem.
///
/// # Examples
///
/// ```
/// use incremental_lang::{Builder, Document, Error, Grammar, Span, Token, TokenKind};
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
/// let mut doc = Document::new(Chars, "abc")?;
/// match doc.edit(Span::new(2, 9), "") {
///     Err(Error::OutOfBounds { len, .. }) => assert_eq!(len, 3),
///     other => panic!("unexpected {other:?}"),
/// }
/// assert_eq!(doc.text(), "abc"); // unchanged
/// # Ok::<(), Error>(())
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The edit's span ends past the end of the document.
    ///
    /// `span` is the span that was passed and `len` the document's length in
    /// bytes. The caller's idea of the text has drifted from the document's —
    /// typically an edit was applied twice or dropped. Resynchronize, for
    /// instance by replacing the whole text with a single edit.
    OutOfBounds {
        /// The span that was passed to the edit.
        span: Span,
        /// The document's length in bytes.
        len: u32,
    },

    /// An end of the edit's span falls inside a multi-byte UTF-8 character.
    ///
    /// `offset` is the offending byte offset. Edits are byte ranges over the
    /// document's UTF-8 text; a caller that counts in characters or UTF-16 code
    /// units (as the Language Server Protocol does) must convert to byte
    /// offsets first.
    NotCharBoundary {
        /// The byte offset that splits a character.
        offset: u32,
    },

    /// The text would be longer than `u32::MAX` bytes.
    ///
    /// Positions are 32-bit byte offsets, shared with the rest of the `-lang`
    /// family's `Span` type, which caps a document at 4 GiB. `len` is the
    /// length the text would have had.
    TooLarge {
        /// The length in bytes the text would have had.
        len: usize,
    },

    /// The grammar's lexer broke [the lexer contract](crate::Grammar#the-lexer-contract)
    /// on the whole document: its tokens left a gap, overlapped, or ran past
    /// the end of the text.
    ///
    /// `offset` is the first byte where the tokens and the text disagree. This
    /// is a defect in the grammar, not in the input; it is reported rather than
    /// trusted because positions computed from such tokens would be wrong.
    Tokens {
        /// The first byte offset where the tokens do not tile the text.
        offset: u32,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Error::OutOfBounds { span, len } => write!(
                f,
                "edit span {}..{} runs past the end of the document ({len} bytes)",
                span.start().to_u32(),
                span.end().to_u32()
            ),
            Error::NotCharBoundary { offset } => write!(
                f,
                "edit boundary at byte {offset} falls inside a UTF-8 character"
            ),
            Error::TooLarge { len } => write!(
                f,
                "a document of {len} bytes exceeds the {} byte limit of 32-bit offsets",
                u32::MAX
            ),
            Error::Tokens { offset } => write!(
                f,
                "the grammar's lexer did not tile the text: gap, overlap, or overrun at byte {offset}"
            ),
        }
    }
}

impl core::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn test_display_names_offsets() {
        let out = Error::OutOfBounds {
            span: Span::new(2, 9),
            len: 3,
        };
        assert_eq!(
            out.to_string(),
            "edit span 2..9 runs past the end of the document (3 bytes)"
        );
        assert!(
            Error::NotCharBoundary { offset: 7 }
                .to_string()
                .contains("byte 7")
        );
        assert!(
            Error::TooLarge { len: 5_000_000_000 }
                .to_string()
                .contains("5000000000")
        );
        assert!(Error::Tokens { offset: 4 }.to_string().contains("byte 4"));
    }
}
