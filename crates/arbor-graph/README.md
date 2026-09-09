<p align="center">
  <img src="https://raw.githubusercontent.com/Anandb71/arbor/main/docs/assets/arbor-logo.svg" alt="Arbor" width="60" height="60" />
</p>

<h1 align="center">arbor-graph</h1>

<p align="center">
  <strong>Graph engine for Arbor</strong><br>
  <em>The Code Property Graph that LLMs can navigate</em>
</p>

<p align="center">
  <a href="https://crates.io/crates/arbor-graph"><img src="https://img.shields.io/crates/v/arbor-graph?style=flat-square&color=blue" alt="Crates.io" /></a>
  <img src="https://img.shields.io/badge/license-MIT-green?style=flat-square" alt="License" />
</p>

---

## Overview

`arbor-graph` is the heart of [Arbor](https://github.com/Anandb71/arbor). It manages:

- **Graph Schema**: Nodes (code entities) + Edges (relationships)
- **Symbol Table**: Cross-file FQN resolution
- **Persistence**: Sled-backed incremental storage
- **Queries**: Path finding, impact analysis, context retrieval
- **Exact edges**: `PinnedEdge` for edges whose endpoints are already resolved
- **Detached rebuilds**: `ScipTask`, the file-backed handle for a background SCIP refresh

## Features

| Feature | Description |
|---------|-------------|
| `petgraph` core | Stable, fast in-memory graph |
| Global Symbol Table | Resolve imports across files |
| Sled Store | ACID-compliant persistence |
| `find_path` | A* shortest path between nodes |
| Serialization | `bincode` for compact storage |

## Architecture

```
arbor-core (parse) → arbor-graph (store) → arbor-server (expose)
                          ↓
                    ArborGraph
                    ├── nodes: HashMap<NodeId, CodeEntity>
                    ├── edges: Vec<(NodeId, NodeId, EdgeKind)>
                    └── symbol_table: SymbolTable
```

## Usage

This crate is used internally. For most use cases:

```bash
cargo install arbor-graph-cli
```

## Links

- **Main Repository**: [github.com/Anandb71/arbor](https://github.com/Anandb71/arbor)

## Exact edges (`PinnedEdge`)

`resolve_edges()` takes a *name* and infers which definition it means, weighting
the result by confidence. That is the right behaviour for Tree-sitter output and
the wrong behaviour for a compiler-produced index, where the answer is already
known.

`PinnedEdge` carries `CodeNode` IDs rather than names:

```rust
use arbor_graph::{Edge, EdgeKind, GraphBuilder, PinnedEdge};

let mut builder = GraphBuilder::new();
builder.add_nodes(nodes);
builder.add_pinned_edges(vec![PinnedEdge {
    from_id: caller_id,
    to_id: callee_id,
    edge: Edge::new(EdgeKind::Calls),
}]);
let graph = builder.build();   // pinned edges applied after name resolution
```

Available on both `GraphBuilder::add_pinned_edges` (queued, applied by `build()`
and `build_without_resolve()`) and `ArborGraph::add_pinned_edges` (immediate,
returns the count dropped for missing endpoints). Endpoints not in the graph are
dropped rather than turned into placeholder vertices; self-edges are dropped
because they add no reachability and skew centrality toward whatever recurses.

Produced by [`arbor-scip`](../arbor-scip).

## Detached rebuild handles (`ScipTask`)

A background SCIP rebuild runs in a different process from the MCP bridge, so an
in-memory task registry cannot see it. `ScipTask` persists the handle to
`.arbor/scip-task.json` (atomically, via temp file plus rename) so the CLI, the
bridge, and an agent all poll the same record. `worker_alive()` asks the OS
whether the worker PID is still running rather than inferring it from the
record's age.
