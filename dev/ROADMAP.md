# incremental-lang - Roadmap

> Path from scaffold to a stable 1.0. Hard parts are front-loaded; each phase has hard exit criteria.
> Master plan: ../../_strategy/LANG_COLLECTION.md
>
> **Anti-deferral rule:** no listed hard task moves to a later phase unless this file records the move and the reason.

## v0.1.0 - Scaffold (DONE)
Compiles, CI green, structure correct, no domain logic.
- [x] Manifest, README, CHANGELOG, REPS, dual license, CI, deny, clippy, rustfmt.

## v0.2.0 - Core (THE HARD PART, NOT DEFERRED) (DONE)
Incremental reparse: reparse only the changed region for fast editor tooling.
Dependencies (wires parser, syntax) are wired here, when first used.
Exit criteria:
- [x] Every public item has rustdoc + a runnable example.
- [x] Core invariants property-tested (full DIRECTIVES + API authored at this stage).

Delivered 2026-10-07: `Grammar`, `Builder` / `Checkpoint`, `Document`,
`Reparse`, `Stats`, `Tree` / `Node` / `Element`, `Error`. The tree is an
`Arc`-shared green tree of widths; an edit reparses the innermost reparsable
node strictly containing it, guarded by generic checks (neighbouring tokens
relex unchanged, edge tokens keep their kinds, one node of the same kind is
rebuilt without the grammar looking past its end), widens within a budget of
one full parse, and splices copy-on-write. The invariant — the tree after any
edit equals a fresh parse — is property-tested, including snapshot isolation.
Scaffold defects fixed on the way in: unquoted `keywords` / `categories` in
`Cargo.toml` (the manifest did not parse), clippy MSRV `1.87` → `1.85`, a
stale project name in `deny.toml`, BOM/CRLF in docs, and a dead link to
`dev/DIRECTIVES.md`.

Dependency wiring, recorded under the anti-deferral rule:

- **`syntax-lang` — wired.** Its `Token`, `TokenKind`, and `Span` are the types
  the grammar contract is written in (re-exported so versions always match),
  and `Tree::to_syntax` converts a document's tree into its `Node` for tooling
  built on the family's standard CST. The incremental tree itself cannot *be*
  a `syntax_lang::Node`: that type stores absolute spans in private fields, so
  every node after an edit would have to be rebuilt to shift its positions —
  the whole cost incremental reparsing exists to avoid. The live tree stores
  widths instead, and converts on demand.
- **`parser-lang` — not wired, by design.** Its `Parser` is built for one-shot
  parsing into values: it skips trivia without handing it to a tree, reports
  errors as side-channel `diag-lang` diagnostics with absolute spans that every
  splice would have to shift or discard, and its Pratt driver combines operand
  values rather than letting the grammar open a node around an operand already
  built. Incremental reparsing needs a cursor fused with a tree builder that
  owns trivia placement (so node spans are exact), records whether the grammar
  looked past the end of the node being reparsed (the generic safety check),
  and opens nodes retroactively — that is `Builder`. Errors are recorded as
  nodes in the tree, so they move with it. This is a design decision, not a
  deferral.
- **`serde` and `loom` — removed.** The scaffold declared both with no code
  behind them. The document is edited through `&mut self` and shares only
  immutable snapshots, so there is no concurrent code for `loom` to check.

## v1.0.0 - API freeze (DONE)
Public surface stable and frozen until 2.0.
- [x] docs/API.md marked stable; SemVer promise recorded.
- [x] Full test + benchmark suite green on all three platforms.

Shipped 2026-10-07. Before freezing, an adversarial review attacked the
equivalence guarantee with grammars written to the documented contract. Two
findings were gaps in the contract's wording rather than in the engine, and
the contract now states them: a lexer may look at most one character past each
token (scan-ahead-and-back-off lexers are out), and nothing outside a
reparsable node may depend on its contents beyond its first token. Neither is
observable from a single reparse, so they are the grammar's to keep. The
review also found an edit path quadratic in the depth of non-reparsable
nesting (fixed: per-level facts are recorded on one walk down), trivia leaking
into a node opened at a fresh checkpoint (fixed), and counters advanced by a
caught grammar panic (fixed). The API is unchanged from 0.2.0. Its fuzzing of a
contract-compliant grammar — about five million edits, including empty nodes
at node edges, zero-width tokens, checkpoints, and multi-byte text — found no
divergence. The surface, the equivalence guarantee, the grammar contract, the
builder semantics, and MSRV 1.85 are recorded as the contract in
`docs/API.md#stability`. Tests and benchmarks green on Windows and Linux (WSL2)
locally; macOS through the CI matrix.

Additive 1.x candidates (not commitments):

- Token text in `Builder` (`peek_text`) for contextual keywords. If added, the
  reparse checks must compare edge and neighbour token *text*, not just kind
  and width, since an edit inside a token can keep its kind and change its
  text.
- A covering-node lookup (`Node::covering(offset)`) for hover and selection.
- An optional, provided `Grammar` method declaring a lexer's lookahead, used to
  widen the relex window — so lexers that cannot meet the one-character rule
  could still reparse incrementally.
- A `serde` feature for `Stats`.
