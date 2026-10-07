<h1 align="center">
    <img width="99" alt="Rust logo" src="https://raw.githubusercontent.com/jamesgober/rust-collection/72baabd71f00e14aa9184efcb16fa3deddda3a0a/assets/rust-logo.svg">
    <br>
    <b>incremental-lang</b>
    <br>
    <sub><sup>INCREMENTAL REPARSE</sup></sub>
</h1>

<div align="center">
    <a href="https://crates.io/crates/incremental-lang"><img alt="Crates.io" src="https://img.shields.io/crates/v/incremental-lang"></a>
    <a href="https://crates.io/crates/incremental-lang"><img alt="Downloads" src="https://img.shields.io/crates/d/incremental-lang?color=%230099ff"></a>
    <a href="https://docs.rs/incremental-lang"><img alt="docs.rs" src="https://img.shields.io/docsrs/incremental-lang"></a>
    <a href="https://github.com/jamesgober/incremental-lang/actions"><img alt="CI" src="https://github.com/jamesgober/incremental-lang/actions/workflows/ci.yml/badge.svg"></a>
    <a href="https://github.com/rust-lang/rfcs/blob/master/text/2495-min-rust-version.md"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.85%2B-blue"></a>
</div>

<br>

<div align="left">
    <p>
        <strong>incremental-lang</strong> keeps a source text and its lossless concrete syntax tree in step while the text is being edited, rebuilding only the part of the tree around each change. It is the reparsing core an editor or language server needs: report an edit, and the tree is current again in microseconds, whatever the size of the file.
    </p>
    <p>
        An editor reparses on every keystroke. Parsing the whole file each time costs time in proportion to the file; reparsing incrementally costs time in proportion to the edit. incremental-lang finds the innermost node around a change that can be reparsed on its own &mdash; a block, an object, an argument list &mdash; relexes and reparses just that node, and splices the result into a tree that shares every untouched subtree with the previous version. Every shortcut is checked: when a reparse could disagree with a fresh parse (an edit that unbalances brackets, opens a comment, or lets a token grow across a boundary) the document widens the reparse instead. The tree after an edit is always the tree a parse from scratch would produce. The crate owns no grammar: a language supplies its lexer and a recursive-descent parser written against a small builder, and incremental-lang does the rest.
    </p>
    <br>
    <hr>
    <p>
        <strong>MSRV is 1.85+</strong> (Rust 2024 edition). <code>no_std</code>-compatible (needs only <code>alloc</code>), <code>#![forbid(unsafe_code)]</code>, built on <a href="https://crates.io/crates/syntax-lang"><code>syntax-lang</code></a>.
    </p>
    <blockquote>
        <strong>1.0.0 is the API freeze.</strong> The public surface is stable and follows Semantic Versioning &mdash; no breaking changes before <code>2.0</code>. See <a href="./docs/API.md#stability"><code>docs/API.md</code></a> for the frozen-surface list and the SemVer promise, and <a href="./CHANGELOG.md"><code>CHANGELOG.md</code></a>.
    </blockquote>
</div>

<hr>
<br>

## The model

A language plugs in once; a document does the rest.

- A **[`Grammar`](./docs/API.md#grammar)** is what a language supplies: one kind type naming both nodes and tokens, a lexer, a parser, and the list of node kinds that may be reparsed on their own.
- A **[`Builder`](./docs/API.md#builder)** is what the parser works with: a cursor over the significant tokens and a tree builder in one. Trivia never reaches the grammar; the builder threads it into the tree, so the tree is lossless whatever the grammar does.
- A **[`Document`](./docs/API.md#document)** owns the text and its tree. **[`Document::edit`](./docs/API.md#documentedit)** applies a change and reports, as a **[`Reparse`](./docs/API.md#reparse)**, which part of the tree it rebuilt; **[`Stats`](./docs/API.md#stats)** counts the work.
- A **[`Tree`](./docs/API.md#tree)** is an immutable snapshot: cloning it is one reference-count increment, and it can be sent to another thread. It is read through positioned, zero-copy **[`Node`](./docs/API.md#node)** and **[`Element`](./docs/API.md#element)** views, and converts to a [`syntax_lang::Node`](https://docs.rs/syntax-lang) for tools built on the family's standard CST.

<br>

What happens on an edit:

| Step | What the document does |
|---|---|
| **Locate** | Walks down from the root to the innermost node that *strictly* contains the edit, noting every reparsable node on the way. |
| **Relex** | Lexes that node's new text together with the token on each side of it. |
| **Check** | Requires the neighbouring tokens to come back unchanged, the node's first and last tokens to keep their kinds, and the grammar to rebuild one node of the same kind without looking past its end. |
| **Splice** | Replaces the node in place, sharing every other subtree. A failed check moves on to the next node outward, and finally to a full reparse. |

<hr>
<br>

## Installation

```toml
[dependencies]
incremental-lang = "1"
```

Or from the terminal:

```bash
cargo add incremental-lang
```

`Token`, `TokenKind`, and `Span` are re-exported, so a grammar needs no other dependency. MSRV: Rust 1.85 (Rust 2024 edition).

<hr>
<br>

## Quick start

A grammar for nested lists such as `(add (mul 2 3) 4)`, with every list reparsable on its own:

```rust
use incremental_lang::{Builder, Document, Grammar, Span, Token, TokenKind};

// One kind type names both the nodes (Root, List) and the tokens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum K { Root, List, Open, Close, Atom, Space }

impl TokenKind for K {
    fn is_trivia(&self) -> bool { matches!(self, K::Space) }
}

struct Lists;

impl Grammar for Lists {
    type Kind = K;
    const ROOT: K = K::Root;

    fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
        let bytes = text.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let start = i;
            let kind = match bytes[i] {
                b'(' => { i += 1; K::Open }
                b')' => { i += 1; K::Close }
                b' ' => {
                    while i < bytes.len() && bytes[i] == b' ' { i += 1; }
                    K::Space
                }
                _ => {
                    while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; }
                    K::Atom
                }
            };
            tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
        }
    }

    fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
        if kind == K::List {
            return list(b);
        }
        b.start(K::Root);
        while !b.at_end() {
            if b.at(K::Open) { list(b) } else { b.bump() }
        }
        b.finish();
    }

    fn is_reparsable(&self, kind: K) -> bool {
        kind == K::List
    }
}

fn list(b: &mut Builder<'_, K>) {
    b.start(K::List);
    b.bump(); // `(`
    // Test for the closer before the end: a balanced list then never looks
    // past its own `)`, which is what makes it safe to reparse alone.
    while !b.at(K::Close) && !b.at_end() {
        if b.at(K::Open) { list(b) } else { b.bump() }
    }
    b.eat(K::Close);
    b.finish();
}

let mut doc = Document::new(Lists, "(add (mul 2 3) 4)")?;

// Typing inside `(mul 2 3)` rebuilds only that list.
let edit = doc.edit(Span::new(12, 13), "30")?;
assert!(!edit.is_full());
assert_eq!(edit.span(), Span::new(5, 15));
assert_eq!(doc.text(), "(add (mul 2 30) 4)");

// Deleting a `)` unbalances the text; the document reparses all of it.
assert!(doc.edit(Span::new(14, 15), "")?.is_full());

// Either way, the tree is the tree a fresh parse produces.
assert_eq!(doc.tree(), Document::new(Lists, doc.text())?.tree());
# Ok::<(), incremental_lang::Error>(())
```

<br>

### Reading the tree

Nodes carry absolute spans and borrow their text from the source, so reading the tree allocates nothing. Clone the tree to keep a version of it: the clone is one reference-count increment and never changes.

```rust
use incremental_lang::{Builder, Document, Element, Grammar, Span, Token, TokenKind};
# #[derive(Clone, Copy, Debug, PartialEq, Eq)]
# enum K { Root, List, Open, Close, Atom, Space }
# impl TokenKind for K { fn is_trivia(&self) -> bool { matches!(self, K::Space) } }
# struct Lists;
# impl Grammar for Lists {
#     type Kind = K;
#     const ROOT: K = K::Root;
#     fn lex(&self, text: &str, tokens: &mut Vec<Token<K>>) {
#         let bytes = text.as_bytes();
#         let mut i = 0;
#         while i < bytes.len() {
#             let start = i;
#             let kind = match bytes[i] {
#                 b'(' => { i += 1; K::Open }
#                 b')' => { i += 1; K::Close }
#                 b' ' => { while i < bytes.len() && bytes[i] == b' ' { i += 1; } K::Space }
#                 _ => { while i < bytes.len() && !b"() ".contains(&bytes[i]) { i += 1; } K::Atom }
#             };
#             tokens.push(Token::new(kind, Span::new(start as u32, i as u32)));
#         }
#     }
#     fn parse(&self, kind: K, b: &mut Builder<'_, K>) {
#         if kind == K::List { return list(b); }
#         b.start(K::Root);
#         while !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#         b.finish();
#     }
#     fn is_reparsable(&self, kind: K) -> bool { kind == K::List }
# }
# fn list(b: &mut Builder<'_, K>) {
#     b.start(K::List);
#     b.bump();
#     while !b.at(K::Close) && !b.at_end() { if b.at(K::Open) { list(b) } else { b.bump() } }
#     b.eat(K::Close);
#     b.finish();
# }

let mut doc = Document::new(Lists, "(a (b c))")?;
let snapshot = doc.tree().clone();

// Every list, with its text.
let lists: Vec<&str> = doc
    .tree()
    .root()
    .descendants()
    .filter(|node| node.kind() == K::List)
    .filter_map(|node| node.text(doc.text()))
    .collect();
assert_eq!(lists, ["(a (b c))", "(b c)"]);

// The atoms directly inside the outer list.
let outer = doc.tree().root().children().find_map(Element::as_node).ok_or("no list")?;
let atoms = outer.children().filter(|child| child.kind() == K::Atom).count();
assert_eq!(atoms, 1);

// Editing leaves the snapshot alone.
doc.edit(Span::new(6, 7), "x y")?;
assert_eq!(snapshot.root().span(), Span::new(0, 9));
assert_eq!(doc.tree().root().span(), Span::new(0, 11));
# Ok::<(), Box<dyn std::error::Error>>(())
```

<hr>
<br>

## Examples

Three runnable examples ship in [`examples/`](./examples). They share a complete grammar for JSON with comments in [`examples/common/json.rs`](./examples/common/json.rs) — a hand-written lexer with one character of lookahead, a recursive-descent parser with error recovery, and objects and arrays marked reparsable. It is the template for wiring a real language into the crate.

- **Editor** — a settings file edited keystroke by keystroke: a new setting typed one character at a time, a typo and its backspace, a paste, and a stray `/*`. Each line shows what was rebuilt; the summary compares the bytes lexed against reparsing the file on every keystroke.
  ```bash
  cargo run --example editor
  ```
- **Diagnostics** — live syntax errors read straight from the tree. Errors are nodes, so they move with incremental edits and there is no separate list to keep in step; the example prints caret diagnostics as mistakes are typed and fixed.
  ```bash
  cargo run --example diagnostics
  ```
- **Snapshots** — an editing thread publishes a snapshot of every version to a background worker over a channel, then hands the final tree to `syntax-lang` with `Tree::to_syntax`.
  ```bash
  cargo run --example snapshots
  ```

<hr>
<br>

## Performance

The cost of an edit is the cost of relexing and reparsing one small node, plus a walk down the tree and an in-place splice; it does not grow with the size of the file. The tree stores widths rather than positions, so a subtree after the edit is still correct at its new offset and is reused untouched. When the document is the only owner of its tree, the path to the edited node is updated in place; while a snapshot shares it, the path is copied instead and the snapshot never changes. Every working buffer is pooled, so a steady editing session allocates only the nodes it rebuilds.

Measured with the benchmarks in [`benches/`](./benches) on generated JSON (objects grouped into arrays of 32), x86_64, Rust stable, release profile. Each edit row is one keystroke *and* its backspace — two edits:

| Benchmark | What it measures | Windows | Linux (WSL2) |
|---|---|---:|---:|
| `edit/10KB` | Keystroke pair inside a small object, incremental. | ~1.3 µs | ~0.76 µs |
| `edit/100KB` | The same in a 100 KB document. | ~2.2 µs | ~1.2 µs |
| `edit/1MB` | The same in a 1 MB document. | ~11 µs | ~7.0 µs |
| `edit_full/1MB` | The same pair with nothing reparsable: two full reparses. | ~41 ms | ~21 ms |
| `edit_flat/1MB` | Keystroke pair inside one element of a 7,700-element flat array. | ~32 µs | ~25 µs |
| `edit_deep/40000` | Two edits in a small list beneath a 40,000-deep chain of plain nodes. | ~0.90 ms | ~0.58 ms |
| `parse/1MB` | `Document::new` on 1 MB: lex and parse everything, then free it. | ~21 ms | ~9.0 ms |
| `walk/tokens/100KB` | Iterating every token of a 100 KB tree. | ~151 µs | ~120 µs |
| `snapshot/clone` | Keeping a version of the tree. | ~14 ns | ~13 ns |

At 1 MB an incremental keystroke is roughly three thousand times cheaper than reparsing the file. What remains at that size is mostly the text itself: the document stores one contiguous `String`, and inserting into the middle of it moves the half after the edit. Whole-document parsing allocates once per node, which is why it is noticeably faster on Linux than with the default Windows allocator; it is the path taken when a file is opened, not on each keystroke.

Run them yourself:

```bash
cargo bench --bench bench
```

Criterion writes per-benchmark reports to `target/criterion/`. Numbers vary by CPU; use the trend across runs, not a single absolute.

<hr>
<br>

## Design notes

- **Green tree, positioned views.** Nodes store their width and their children, never their position, and are shared by `Arc`. A node's kind and width live in its parent's child array, so each node is a single allocation and finding the child that covers an offset reads one contiguous array. Absolute positions are computed on the way down by the `Node` view.
- **Copy-on-write splicing.** An edit replaces one node and adjusts the widths on the path above it. The path is updated in place when no snapshot shares it, and copied when one does — snapshots are immutable without making the common case pay for them.
- **Reparse only what is safe.** A node is reparsed alone only if the edit is strictly inside it, the tokens on either side relex unchanged, its edge tokens keep their kinds, and the grammar rebuilds exactly one node of the same kind without looking past the node's last token. That last check catches unbalanced brackets generically, with no bracket table: a list that lost its `)` keeps asking for more tokens. Two rules cannot be checked from a single reparse and are part of the grammar contract instead: the lexer looks at most one character past each token, and no parsing decision outside a reparsable node depends on what is inside it beyond its first token. The property tests hold every edit to a fresh parse.
- **Bounded failure cost.** Failed single-node attempts move outward to larger nodes, but never spend more than one whole-document parse's worth of lexing before reparsing everything, so an edit never costs more than about twice a full parse.
- **Trivia placed by the builder.** The grammar never sees whitespace or comments. Trivia before a node goes to the enclosing node, so nodes begin and end on significant tokens and their spans are exactly their syntax — which is what makes a node's text reparsable on its own. An empty node (a marker for something missing) sits right after the last token before it.
- **Errors in the tree.** Grammars record syntax errors as nodes — around an unexpected token, or empty where something is missing — so diagnostics move with the tree under incremental edits and need no separate bookkeeping.
- **Transactional edits, no panics on input.** Out-of-range edits, edits that split a UTF-8 character, and lexers that fail to tile their input are reported as errors. The text and the tree are replaced together, only once the new tree is complete, so a refused edit — or even a panic in the grammar's own code — leaves the document exactly as it was. Traversal, comparison, and teardown are iterative, so deeply nested trees cannot overflow the stack inside the crate.

<hr>
<br>

## Testing

The suite runs on Windows, Linux (WSL2 Ubuntu), and macOS through the CI matrix, on stable and the 1.85 MSRV:

```bash
cargo test                       # unit + integration + property + doctests
cargo clippy --all-targets --all-features -- -D warnings
cargo bench --bench bench
```

The property tests in [`tests/proptests.rs`](./tests/proptests.rs) hold the central invariant: after any sequence of edits, the document's tree equals the tree a fresh parse of its text produces. Documents are generated JSON, mostly well formed, so single-node reparses are attempted and taken; edits insert, delete, and replace fragments that include the ones that break lexing (quotes, comment openers) and balance (stray brackets). A second grammar marks more kinds reparsable to give an unsafe splice more chances to slip through, every other edit runs with a snapshot alive to exercise copy-on-write, and a separate test confirms the single-node path is really taken. Every `rust` example in this README and in [`docs/API.md`](./docs/API.md) is compiled and run as a doctest.

<hr>
<br>

## Cross-platform support

- Linux (x86_64, aarch64)
- macOS (x86_64, Apple Silicon)
- Windows (x86_64)

The crate uses no operating-system facilities and no platform-specific code; behaviour is identical on every target with atomic pointer support (needed for the shared tree).

<hr>
<br>

## Contributing

See [`REPS.md`](./REPS.md) for the engineering standards every change is held to, and [`dev/ROADMAP.md`](./dev/ROADMAP.md) for the plan to 1.0. Before a PR: `cargo fmt --all`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all-features` must be clean.

<br>

<div id="license">
    <h2>License</h2>
    <p>Licensed under either of</p>
    <ul>
        <li><b>Apache License, Version 2.0</b> &mdash; <a href="./LICENSE-APACHE">LICENSE-APACHE</a></li>
        <li><b>MIT License</b> &mdash; <a href="./LICENSE-MIT">LICENSE-MIT</a></li>
    </ul>
    <p>at your option.</p>
</div>

<div align="center">
  <h2></h2>
  <sup>COPYRIGHT <small>&copy;</small> 2026 <strong>James Gober <me@jamesgober.com>.</strong></sup>
</div>
