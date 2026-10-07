//! An editing session, keystroke by keystroke.
//!
//! A JSONC settings file is opened and edited the way a person edits it: a new
//! setting typed one character at a time, a typo corrected with backspace, a
//! value replaced by a paste, and finally a careless `/*` that comments out the
//! rest of the file. After each change the example prints what the document
//! rebuilt, and at the end it checks the tree against a fresh parse.
//!
//! ```bash
//! cargo run --example editor
//! ```

#[path = "common/json.rs"]
mod json;

use incremental_lang::{Document, Error, Span};
use json::Json;

const SETTINGS: &str = r#"{
  // Editor settings
  "editor": {
    "fontSize": 14,
    "tabSize": 4,
    "rulers": [80, 100]
  },
  "files": {
    "exclude": ["target", ".git"],
    "trimTrailingWhitespace": true
  }
}
"#;

fn main() -> Result<(), Error> {
    let mut doc = Document::new(Json, SETTINGS)?;
    println!("opened {} bytes\n", doc.text().len());

    // Type a new setting after `"tabSize": 4,`, one keystroke at a time.
    let mut cursor = offset_after(&doc, "\"tabSize\": 4,");
    for c in "\n    \"wordWrap\": \"on\",".chars() {
        let mut buf = [0; 4];
        report(&mut doc, Span::new(cursor, cursor), c.encode_utf8(&mut buf))?;
        cursor += c.len_utf8() as u32;
    }

    // A typo and its correction: `100` becomes `1000`, then backspace.
    let at = offset_after(&doc, "[80, 100");
    report(&mut doc, Span::new(at, at), "0")?;
    report(&mut doc, Span::new(at, at + 1), "")?;

    // Paste over a value.
    let start = offset_after(&doc, "\"exclude\": ");
    let end = offset_after(&doc, "\".git\"]");
    report(
        &mut doc,
        Span::new(start, end),
        "[\"target\", \"node_modules\", \".git\"]",
    )?;

    // A comment opener with no closer: everything after it becomes a comment,
    // which no single node can absorb.
    let at = offset_after(&doc, "\"fontSize\": 14,");
    report(&mut doc, Span::new(at, at), " /*")?;

    let fresh = Document::new(Json, doc.text())?;
    assert_eq!(
        doc.tree(),
        fresh.tree(),
        "incremental tree must match a fresh parse"
    );

    let stats = doc.stats();
    println!(
        "\n{} edits: {} served by a single node, {} by a full reparse",
        stats.edits,
        stats.partial,
        stats.full - 1
    );
    println!(
        "lexed {} bytes in total, against {} for reparsing the file on every edit",
        stats.bytes,
        (stats.edits + 1) * doc.text().len() as u64
    );
    println!("the tree matches a fresh parse of the final text");
    Ok(())
}

/// Applies one edit and prints a line describing what was rebuilt.
fn report(doc: &mut Document<Json>, span: Span, text: &str) -> Result<(), Error> {
    let reparse = doc.edit(span, text)?;
    let rebuilt = reparse.span();
    let what = if reparse.is_full() {
        "full reparse"
    } else {
        "node"
    };
    let excerpt: String = doc.text()[rebuilt.start().to_usize()..rebuilt.end().to_usize()]
        .chars()
        .map(|c| if c == '\n' { ' ' } else { c })
        .take(44)
        .collect();
    println!(
        "{:>14} -> {what:<12} {:?} {}..{}  {excerpt}",
        format!("{text:?}"),
        reparse.kind(),
        rebuilt.start().to_u32(),
        rebuilt.end().to_u32(),
    );
    Ok(())
}

/// The byte offset just past the first occurrence of `needle`.
fn offset_after(doc: &Document<Json>, needle: &str) -> u32 {
    let at = doc.text().find(needle).unwrap_or(0) + needle.len();
    at as u32
}
