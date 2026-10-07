<h1 align="center">
    <img width="90px" height="auto" src="https://raw.githubusercontent.com/jamesgober/jamesgober/main/media/icons/hexagon-3.svg" alt="Triple Hexagon">
    <br><b>CHANGELOG</b>
</h1>
<p>
  All notable changes to <code>incremental-lang</code> will be documented in this file. The format is based on <a href="https://keepachangelog.com/en/1.1.0/">Keep a Changelog</a>,
  and this project adheres to <a href="https://semver.org/spec/v2.0.0.html/">Semantic Versioning</a>.
</p>

---

## [Unreleased]

### Added

### Changed

### Fixed

### Security

---

## [1.0.0] - 2026-10-07

API freeze. The public surface introduced in 0.2.0 is now stable and frozen
under Semantic Versioning: no breaking changes ship before `2.0`. Before the
freeze, an adversarial review of the reparse logic tested the central promise
— that the tree after every edit equals a fresh parse — against grammars
written to the documented contract. It found two gaps in the contract's
wording and one performance bug, all corrected here; the API itself is
unchanged.

### Added

- The `edit_deep` benchmark (an edit at the bottom of a deep chain of nodes)
  and regression tests for the three fixes.

### Changed

- Bumped the crate version to `1.0.0` and declared the public API stable.
  `docs/API.md` is marked stable and records the SemVer promise: the frozen
  surface, the equivalence guarantee, a grammar contract that will not grow,
  the builder semantics, transactional edits, the meaning of every `Stats`
  field and `Error` variant, `syntax-lang` as a public dependency, and MSRV
  1.85 as a compatibility surface.
- The `Grammar` lexer contract now states what single-node reparsing actually
  relies on: besides being restartable, a lexer may look at most one
  character past each token it produces. The previous wording said any
  longest-match lexer qualifies, but a lexer that scans ahead and backs off
  (a `"…"` string that falls back to a lone `"`) can make an incremental tree
  differ from a fresh parse.
- The `Grammar` parser contract now also requires that nothing outside a
  reparsable node depend on its contents beyond its first token — no lookahead
  past that token into the node, and nothing its parse computes fed back into
  the parse around it. Neither can be seen from a single reparse, so the
  documentation now says plainly which rules the document checks and which a
  grammar must keep.
- Crate-level documentation gained a Stability section; the README and
  `docs/API.md` install snippets now use `incremental-lang = "1"`, and `Tree`
  equality is documented as structural (it compares kinds and spans, not
  text).

### Fixed

- An edit beneath a deep chain of nodes that are not reparsable took time
  quadratic in the depth: each candidate level walked down from the root
  again. Everything the candidate loop needs is now recorded on a single walk
  down, so the cost is linear in the depth (40,000 levels: under half a
  millisecond per edit instead of seconds).
- `Builder::start_at` with a checkpoint taken after the last consumed token
  could open a node that began with the trivia before its first token,
  contradicting the documented trivia placement. A checkpoint now remembers
  whether a token has been consumed since, and a node opened there with
  nothing consumed in between places that trivia outside, as `start` does.
- A panic in grammar code during a single-node attempt, caught by the caller,
  left `Stats::bytes` and `Stats::rejected` advanced. Counters for an edit are
  now committed only when the edit succeeds.

---

## [0.2.0] - 2026-10-07

The core, and the hard part of the roadmap: the scaffold becomes a working
incremental reparser. A document keeps a text and its lossless concrete syntax
tree in step under edits, rebuilding only the innermost node around each
change that can safely be reparsed on its own, and the tree after every edit is
the tree a fresh parse would produce. A language plugs in through one trait and
a small cursor-and-builder; the crate supplies the rest.

### Added

- `Grammar` — what a language supplies: one kind type for nodes and tokens
  (`Kind`), the root kind (`ROOT`), a lexer (`lex`), a parser (`parse`), and the
  node kinds that may be reparsed on their own (`is_reparsable`, default none).
  The lexer and parser contracts are documented on the trait.
- `Builder` and `Checkpoint` — the cursor and tree builder a grammar parses
  with: `peek`, `nth`, `at`, `at_end`, `bump`, `eat`, `start`, `finish`,
  `checkpoint`, and `start_at`. Trivia is hidden from the grammar and placed by
  the builder, so nodes begin and end on significant tokens and the tree is
  lossless whatever the grammar does.
- `Document` — the text and its tree: `new`, `edit`, `text`, `tree`, `grammar`,
  and `stats`. Edits are byte ranges plus replacement text; each is
  transactional and returns a `Reparse` describing what was rebuilt.
- Single-node reparsing with generic safety checks: the edit must lie strictly
  inside the node, the tokens on either side must relex unchanged, the node's
  edge tokens must keep their kinds, and the grammar must rebuild exactly one
  node of the same kind without looking past its end. Failed attempts widen to
  enclosing nodes within a budget of one full parse's worth of lexing, then
  fall back to a whole-document parse.
- `Tree`, `Node`, `Element` — the syntax tree as an `Arc`-shared green tree of
  widths: `O(1)` to clone, `Send + Sync`, structurally comparable, and read
  through positioned, zero-copy views (`kind`, `span`, `text`, `children`,
  `descendants`, `tokens`, `as_node`, `as_token`). Edits update the path to the
  changed node in place when no snapshot shares it, and copy it when one does.
- `Tree::to_syntax` — conversion to `syntax_lang::Node`, the family's standard
  CST, for tooling built on `syntax-lang`.
- `Reparse` and `Stats` — per-edit and running observability: what was
  rebuilt, how many edits were partial or full, how many attempts were refused,
  and how many bytes were lexed.
- `Error` — `OutOfBounds`, `NotCharBoundary`, `TooLarge`, and `Tokens`, each
  carrying the offsets needed to act on it.
- Re-exports of `Span`, `Token`, and `TokenKind` from `syntax-lang`.
- Examples: `editor` (a keystroke-by-keystroke session), `diagnostics` (live
  syntax errors read from the tree), and `snapshots` (versions handed to a
  worker thread, then to `syntax-lang`), sharing a complete JSON-with-comments
  grammar in `examples/common/json.rs`.
- Integration tests, property tests holding every edit to a fresh parse
  (including snapshot isolation), and Criterion benchmarks for full parses,
  incremental edits, wide nodes, traversal, and snapshots.
- `README.md` and `docs/API.md` rust examples run as doctests.

### Changed

- Wired `syntax-lang` 1 as the only dependency.
- `Cargo.toml` description, keywords, and categories describe the crate.
- Removed the scaffold's `serde` feature and `loom` dev-dependency, which had
  no code behind them.

### Fixed

- `Cargo.toml` listed `keywords` and `categories` unquoted, so the manifest
  did not parse.
- `clippy.toml` declared MSRV 1.87 against the crate's 1.85.
- `deny.toml` named another project in its header.
- `dev/ROADMAP.md` and `docs/API.md` carried byte-order marks and CRLF line
  endings.
- The README linked a `dev/DIRECTIVES.md` that does not exist.

---

## [0.1.0] - 2026-06-18

Initial scaffold and repository bootstrap. No domain logic yet &mdash; this release establishes the structure, tooling, and quality gates the implementation will be built on.

### Added

- `Cargo.toml` with crate metadata, Rust 2024 edition, MSRV 1.85.
- Dual `Apache-2.0 OR MIT` license files.
- `README.md`, `CHANGELOG.md`, and a documentation skeleton.
- `REPS.md` compliance baseline.
- `.github/workflows/ci.yml` CI matrix; `deny.toml`, `clippy.toml`, `rustfmt.toml`.
- `dev/DIRECTIVES.md` and `dev/ROADMAP.md` (committed engineering standards + plan).

[Unreleased]: https://github.com/jamesgober/incremental-lang/compare/v1.0.0...HEAD
[1.0.0]: https://github.com/jamesgober/incremental-lang/compare/v0.2.0...v1.0.0
[0.2.0]: https://github.com/jamesgober/incremental-lang/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/jamesgober/incremental-lang/releases/tag/v0.1.0
