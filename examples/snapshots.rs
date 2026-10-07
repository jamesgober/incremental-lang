//! Snapshots for background work, and hand-off to `syntax-lang`.
//!
//! A language server edits on one thread and analyses on others. Cloning a
//! `Tree` is one reference-count increment, the clone never changes, and it is
//! `Send + Sync`, so the editing thread can pass each version to a worker and
//! keep going. Consecutive versions share every subtree the edit between them
//! did not touch.
//!
//! This example runs an editing loop on the main thread and an "outline"
//! worker that receives snapshots over a channel, then converts the final tree
//! into a `syntax_lang::Node` — the family's standard CST — for tooling built
//! on that crate.
//!
//! ```bash
//! cargo run --example snapshots
//! ```

#[path = "common/json.rs"]
mod json;

use std::sync::mpsc;
use std::thread;

use incremental_lang::{Document, Error, Node, Span, Tree};
use json::{Json, Kind};

/// What the editing thread sends: a version number, the tree, and the text it
/// describes (trees hold no text).
struct Version {
    number: usize,
    tree: Tree<Kind>,
    text: String,
}

fn main() -> Result<(), Error> {
    let mut doc = Document::new(Json, "{\n  \"servers\": []\n}\n")?;
    let (sender, receiver) = mpsc::channel::<Version>();

    // The worker outlines each version: the value of every `"name"` member.
    let worker = thread::spawn(move || {
        for version in receiver {
            let names: Vec<&str> = version
                .tree
                .root()
                .descendants()
                .filter(|node| node.kind() == Kind::Member)
                .filter_map(|member| name_value(member, &version.text))
                .collect();
            println!("v{}: {} server(s) {names:?}", version.number, names.len());
        }
    });

    // The editing thread adds servers one at a time, publishing each version.
    let servers = ["alpha", "beta", "gamma"];
    for (number, name) in servers.iter().enumerate() {
        let at = doc.text().find(']').unwrap_or(0) as u32;
        let separator = if number == 0 { "" } else { ", " };
        let entry = format!(
            "{separator}{{\"name\": \"{name}\", \"port\": {}}}",
            8000 + number
        );
        doc.edit(Span::new(at, at), &entry)?;

        let version = Version {
            number: number + 1,
            tree: doc.tree().clone(), // O(1)
            text: doc.text().to_string(),
        };
        if sender.send(version).is_err() {
            break; // the worker is gone; nothing left to publish to
        }
    }
    drop(sender);
    if worker.join().is_err() {
        eprintln!("the outline worker panicked");
    }

    // Hand the final tree to tooling built on syntax-lang.
    let cst = doc.tree().to_syntax();
    let significant = cst.tokens().filter(|token| !token.is_trivia()).count();
    println!(
        "\nsyntax-lang CST: {:?} over bytes {}, {} significant tokens",
        cst.kind(),
        cst.span(),
        significant
    );
    assert_eq!(cst.text(doc.text()), Some(doc.text()));
    Ok(())
}

/// For a member `"name": "value"`, the unquoted value.
fn name_value<'t>(member: Node<'_, Kind>, text: &'t str) -> Option<&'t str> {
    let mut tokens = member.tokens().filter(|token| !token.is_trivia());
    let key = tokens.next()?;
    let _colon = tokens.next()?;
    let value = tokens.next()?;
    let slice = |span: Span| text.get(span.start().to_usize()..span.end().to_usize());
    if slice(key.span)? != "\"name\"" {
        return None;
    }
    slice(value.span)?.strip_prefix('"')?.strip_suffix('"')
}
