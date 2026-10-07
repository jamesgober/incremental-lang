//! Live syntax errors from the tree.
//!
//! The JSONC grammar records problems in the tree itself: an `Error` node
//! around a token that does not belong, and an empty `Error` node where
//! something is missing. Because errors live in the tree, they move with it —
//! an incremental reparse updates them along with everything else, and there
//! is no separate diagnostics list to keep in step.
//!
//! This example types a small document with mistakes and fixes them, printing
//! the errors after each step the way an editor would underline them.
//!
//! ```bash
//! cargo run --example diagnostics
//! ```

#[path = "common/json.rs"]
mod json;

use incremental_lang::{Document, Error, Node, Span};
use json::{Json, Kind};

fn main() -> Result<(), Error> {
    let mut doc = Document::new(Json, "{\n  \"name\": \"demo\",\n  \"ports\": [80 443]\n}\n")?;
    show("opened", &doc);

    // Add the missing comma between the ports.
    let at = find(&doc, "80") + 2;
    doc.edit(Span::new(at, at), ",")?;
    show("added `,` after 80", &doc);

    // Start a new member and stop half-way: the key is there, the value is not.
    let at = find(&doc, "443]") + 4;
    doc.edit(Span::new(at, at), ",\n  \"debug\":")?;
    show("typed `\"debug\":`", &doc);

    // A typo in the value: an unknown word.
    let at = find(&doc, "\"debug\":") + 8;
    doc.edit(Span::new(at, at), " ture")?;
    show("typed ` ture`", &doc);

    // Fix the typo.
    let at = find(&doc, "ture");
    doc.edit(Span::new(at, at + 4), "true")?;
    show("fixed to `true`", &doc);

    let stats = doc.stats();
    println!(
        "{} edits, {} served by a single node",
        stats.edits, stats.partial
    );
    Ok(())
}

/// Prints every syntax error in the document with a caret under it.
fn show(step: &str, doc: &Document<Json>) {
    println!("--- {step}");
    let errors: Vec<Node<'_, Kind>> = doc
        .tree()
        .root()
        .descendants()
        .filter(|node| node.kind() == Kind::Error)
        .collect();
    if errors.is_empty() {
        println!("no errors\n");
        return;
    }
    for error in errors {
        let span = error.span();
        let (line, column) = line_column(doc.text(), span.start().to_usize());
        let source_line = doc.text().lines().nth(line).unwrap_or("");
        let message = match error.text(doc.text()) {
            Some("") | None => "something is missing here".to_string(),
            Some(found) => format!("unexpected `{found}`"),
        };
        println!("{}:{}: {message}", line + 1, column + 1);
        println!("    {source_line}");
        let width = span.len().max(1) as usize;
        println!("    {}{}", " ".repeat(column), "^".repeat(width));
    }
    println!();
}

/// Zero-based line and byte column of `offset`.
fn line_column(text: &str, offset: usize) -> (usize, usize) {
    let before = &text[..offset];
    let line = before.matches('\n').count();
    let column = offset - before.rfind('\n').map_or(0, |i| i + 1);
    (line, column)
}

/// The byte offset of the first occurrence of `needle`.
fn find(doc: &Document<Json>, needle: &str) -> u32 {
    doc.text().find(needle).unwrap_or(0) as u32
}
