<p align="center">
  <img src="https://raw.githubusercontent.com/Anandb71/arbor/main/docs/assets/arbor-logo.svg" alt="Arbor" width="60" height="60" />
</p>

<h1 align="center">arbor-scip</h1>

<p align="center">
  <strong>SCIP index ingestion for Arbor</strong><br>
  <em>Compiler-accurate graphs for JVM languages</em>
</p>

<p align="center">
  <a href="https://crates.io/crates/arbor-scip"><img src="https://img.shields.io/crates/v/arbor-scip?style=flat-square&color=blue" alt="Crates.io" /></a>
  <img src="https://img.shields.io/badge/license-MIT-green?style=flat-square" alt="License" />
</p>

---

## Overview

Arbor's own parsers read source text with Tree-sitter. That is fast and needs no
build, but it cannot resolve this:

```java
public void pay() {
    gateway.charge();   // which charge()? depends on the type of `gateway`
}
```

Arbor records that call as the reference `gateway.charge`. No symbol has that
name — `gateway` is a variable, not a type — so it never resolves and **no edge
is created**. On interface-driven JVM code that is most of the call graph.

Answering it requires type inference, which is a compiler's job. Rather than
reimplement javac, this crate consumes [SCIP](https://github.com/scip-code/scip)
indexes produced by [`scip-java`](https://github.com/scip-code/scip-java), which
runs as a compiler plugin — its resolution *is* the compiler's.

## Features

| Feature | Description |
|---------|-------------|
| Resolved method calls | `obj.method()` becomes a real edge instead of no edge |
| Virtual dispatch | Calls to an interface method also reach its implementations |
| `Implements` edges | The override hierarchy, straight from SCIP relationships |
| Distinct overloads | `find()` and `find(+1)` stay separate nodes, not one merged vertex |
| Honest confidence | Compiler-proven edges are `1.0`; only dispatch expansion goes lower |
| Both range encodings | Typed `single_line_range`/`multi_line_range` (what current `scip-java` emits) and the deprecated `repeated int32 range` |

## Modules

| Module | Responsibility |
|--------|----------------|
| `ingest` | Two-pass conversion: `Definition` occurrences → `CodeNode`, others → edges typed by the target's kind |
| `symbols` | SCIP symbol strings → qualified name + `NodeKind` |
| `dispatch` | Virtual dispatch expansion from `is_implementation` relationships |
| `ranges` | Range decoding and enclosing-definition attribution |
| `edge` | `SymbolEdge` — an edge still expressed in SCIP symbol strings |
| `error` | `ScipError` |

## Usage

```rust
use arbor_scip::{ingest_file, IngestOptions};
use std::path::Path;

let options = IngestOptions::new("/path/to/repo");
let ingest = ingest_file(Path::new("index.scip"), &options)?;

let mut builder = arbor_graph::GraphBuilder::new();
builder.add_nodes(ingest.nodes);
builder.add_pinned_edges(ingest.edges);
let graph = builder.build();
# Ok::<(), Box<dyn std::error::Error>>(())
```

Multi-module builds emit one index per module. Hand them all to `ingest_files`
so cross-module edges resolve — a symbol defined in module B is only linkable
while B's definitions are in scope.

## Why edges bypass name resolution

`arbor-graph`'s `resolve_edges()` takes a *name* and guesses which definition it
means. Routing a compiler-resolved edge through that would discard the only
thing that makes it better than a guess.

So `ScipIngest::edges` are [`arbor_graph::PinnedEdge`] values, carrying node IDs
rather than names, applied after name resolution has run. `ScipIngest::nodes`
deliberately leave `references` empty for the same reason: populating them would
invite a second, guessed edge for every one already known precisely.

## Virtual dispatch expansion

The part type resolution alone does not buy you. A call to
`PaymentGateway.charge()` lands at runtime in `StripeGateway.charge()`; a graph
that stops at the interface reports a blast radius of one for a change that
reaches every implementation.

- Followed **transitively to depth 4** — `interface → AbstractFoo → FooImpl` is
  three levels and ordinary in Spring code
- Confidence `1/n` for `n` candidates; a single implementation is `1.0`
- Dropped past **8** candidates (`MAX_DISPATCH_FANOUT`) — a `Runnable` in a large
  codebase has hundreds, and "calls one of these" carries no information
- The original edge to the declaring symbol is kept

## Reference counters are exhaustive

`resolved + external + ignored + unattributed + self + without_range` equals
every non-definition occurrence in the index. Asserted in the tests, because an
unaccounted remainder is indistinguishable from a decoding bug — which is
exactly how a deprecated-field regression once produced a zero-node graph while
every test passed.

## Dependencies

- [`scip`](https://crates.io/crates/scip) — bindings from `scip-code/scip`,
  shipping pre-generated protobuf (`build = false`), so no `protoc` is needed
- `protobuf` pinned to `=3.7.2` to match `scip` 0.10's own pin

## Documentation

Full reference, including how to produce an index and the staleness rules:
[docs/SCIP.md](https://github.com/Anandb71/arbor/blob/main/docs/SCIP.md)

## License

MIT
