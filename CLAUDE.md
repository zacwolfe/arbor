# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
# Build
cargo build --workspace

# Test all crates
cargo test --workspace

# Test single crate
cargo test -p arbor-graph
cargo test -p arbor-core

# Test single test by name
cargo test -p arbor-graph -- ranking::tests::test_pagerank_basic

# Lint
cargo clippy --workspace --all-targets --all-features

# Format check
cargo fmt --all -- --check

# Format fix
cargo fmt --all

# Benchmarks (criterion; CI regression gate in .github/workflows/benchmarks.yml)
cargo bench -p arbor-graph

# Release build (CLI binary)
cargo build --locked --release -p arbor-graph-cli

# Run CLI locally
cargo run -p arbor-graph-cli -- <command>
```

## Architecture

Arbor is a **semantic code graph engine** — it parses codebases into a dependency graph and exposes that graph to CLIs, GUIs, WebSocket clients, and AI agents (via MCP).

### Crate Dependency Order

```
arbor-core  →  arbor-graph  →  arbor-watcher
                     │               │
                     ├──→ arbor-scip │
                     │               │
                     └──→ arbor-server ──────┐
                                             │
                     arbor-mcp ──────────────┤
                     arbor-cli ──────────────┘  (also → arbor-scip)
                     arbor-gui ─────────────────→ arbor-{core,graph,watcher}
```

### Crate Roles

**`arbor-core`** — Tree-sitter AST parsing. Extracts functions, classes, structs, imports, and call edges for 9 production languages (Rust, TS/JS, Python, Go, Java, C/C++, C#, Dart) plus 5 fallback parsers. Each language lives in `crates/arbor-core/src/languages/`. `parser_v2.rs` is the active parser; `parser.rs` is legacy.

**`arbor-graph`** — In-memory petgraph + sled persistence. Key modules:
- `builder.rs` — converts parsed nodes/edges into the graph, builds per-file import maps for cross-module edge filtering
- `ranking.rs` — PageRank with 10% weight for test-file callers
- `heuristics.rs` — entry point detection (main, HTTP routes, webhooks, jobs, CLI commands)
- `impact.rs` — blast radius, shortest path (A*)
- `slice.rs` — context trimming (token-aware, tiktoken)
- `symbol_table.rs` — cross-file FQN resolution
- `confidence.rs` — edge confidence scoring
- `store.rs` — sled-backed persistence

**`arbor-scip`** — SCIP index ingestion, for compiler-accurate graphs. Tree-sitter cannot resolve `obj.method()` without type inference; a [SCIP](https://github.com/scip-code/scip) index carries the compiler's own resolution instead. **Language-neutral**: the SCIP grammar does not vary by producer, so one code path reads `scip-java` (Java, Kotlin), `scip-typescript`, `scip-python`, `rust-analyzer scip`, `scip-clang`, `scip-dotnet`, `scip-go`, `scip-ruby`, `scip-php` and `scip-dart`. `arbor scip --background` *invokes* whichever of them the project's markers select — the `scip-java`-only days are over. Eight are verified end to end against a real index (`scip-java`, `rust-analyzer`, `scip-typescript`, `scip-python`, `scip-go`, `scip-dotnet`, `scip-dart`, `scip-php`); `scip-clang` and `scip-ruby` are not, so for those run the indexer yourself and pass its `index.scip`. Every indexer verified so far has exposed a bug that was invisible to the test suite — three of them "installed exactly as documented, reported missing" — so treat a new one as bug-finding, not a checkbox. Modules:
- `ingest.rs` — two-pass conversion: `Definition` occurrences → `CodeNode`, other occurrences → edges typed by the target's kind
- `symbols.rs` — SCIP symbol strings (`semanticdb maven . . com/example/Svc#find().`) → qualified name + `NodeKind`; overloads keep their disambiguator so they stay distinct nodes. `SymbolStyle` holds the only language-dependent parts: the scope separator (`.` vs `::`) and the constructor name (`<init>`, `constructor`, `__init__`, …). Chosen per document from SCIP's `Document.language`, falling back to the symbol's scheme
- `dispatch.rs` — virtual dispatch expansion from `is_implementation` relationships, confidence-weighted `1/n` by fan-out
- `ranges.rs` — SCIP range decoding and enclosing-definition attribution

Feeds the graph through `GraphBuilder::add_pinned_edges` / `ArborGraph::add_pinned_edges`, which bypass name resolution entirely — running compiler-resolved edges through it would discard their precision. Driven by `arbor scip`.

**`arbor-watcher`** — `notify`-based file watcher. Debounces at 100ms, respects `.gitignore`, triggers incremental re-parse of changed files only. Two-tier cache: file-level AST + node-level byte ranges.

**`arbor-server`** — Tokio WebSocket server on `ws://localhost:7432`. JSON-RPC methods: `discover`, `impact`, `context`, `graph.subscribe`, `spotlight`. RwLock-protected shared graph state.

**`arbor-mcp`** — MCP bridge for AI agents (stdio by default, stateless HTTP via `arbor bridge --http --port 3333`). Speaks MCP `2026-07-28` with fallback for `2025-03-26` clients. Twenty tools:
- **Orientation**: `get_map` — ranked, token-budgeted skeleton of the codebase (recommended first call), `get_architecture_overview`
- **Surgical**: `list_entry_points`, `get_callers`, `get_callees`, `get_implementors`, `get_supertypes`, `get_type_usages`, `get_references`, `search_symbols`, `get_file_graph`, `get_node_detail`, `explain_symbol`
- **Broad**: `get_logic_path`, `get_knowledge_path`, `find_path`, `analyze_impact`, `get_blast_radius` (git-diff based), `audit_security`, `batch_query`

Module layout: `lib.rs` (tool dispatch + `tools/list`), `protocol.rs` (version negotiation, caching metadata), `tasks.rs` (Tasks extension — background indexing returns task handles agents can poll during cold start), `apps.rs` (MCP Apps — interactive blast-radius and architecture-map UIs via `ui://arbor/*` resources), `http.rs` (HTTP transport with `Mcp-Method`/`Mcp-Name` header routing), `git.rs`.

All tools emit a standard JSON envelope: `{ok, tool, arbor_version, data, meta: {node_count, suggested_next_tool, suggested_next_args}}`. Error responses use `{ok: false, error}`. `search_symbols` and `get_map` support pagination (`offset`, `limit`, `hasMore`).

**`arbor-cli`** — Clap CLI with ~30 subcommands. Most command logic lives in `src/commands.rs`; `src/audit/` implements the security audit and `src/hook/` implements agent-harness installation (`arbor hook claude`). Entry point: `src/main.rs`. Dispatches to the other crates. Binary name: `arbor` (crate name: `arbor-graph-cli`).
Key features:
- `map . --exclude-test`: ranked, token-budgeted project skeleton (PageRank + entry point detection). Supports `--tokens N`, `--focus "pattern"`, `--focus-changed`, `--json`, `--verbose`.
- `callers`/`callees`/`entry-points`/`file-graph`/`inspect`/`path`: graph query commands matching MCP tools.
- `query "term1|term2" . --exclude-test`: multi-term OR search with test file filtering.
- `diff . --markdown`: formats impact analysis report as color-coded Markdown.
- `check . --markdown`: executes safety threshold validation and prints Markdown PASS/FAIL status.
- `summary .`: auto-generates structured Pull Request descriptions based on graph diff analysis.
- `agent review`/`agent onboard`/`agent guard`: built-in autonomous workflows (PR risk review, contributor onboarding guide, architecture violation check with `--max-blast-radius`).
- `audit "sink"`: traces call paths to sensitive sinks (e.g. `db_query`, `exec`).
- `explain "question"`: graph-backed context slicing for a question (`--tokens`, `--why`).
- `hook claude [--global]`: installs Arbor directives + hooks into Claude Code settings.
- `scip <index.scip>...`: builds the graph from compiler-produced SCIP indexes (`--root`, `--merge`, `--no-dispatch`, `--json`). With `--background` and no index arguments, detects which indexers this project needs (`src/indexers.rs`) and runs them.

**`arbor-gui`** — egui immediate-mode desktop UI. Standalone binary. Uses `arbor_graph::cache` to load a SCIP-provenanced graph rather than re-indexing (it previously always re-parsed with Tree-sitter, so on a JVM project it showed a different graph from the CLI).

### Data Flow

1. `arbor-core` parses files with Tree-sitter → `Node` + call edge structs
2. `arbor-graph`'s `builder.rs` assembles petgraph, builds import maps, filters false cross-module edges
3. `arbor-watcher` detects changes → partial re-parse → graph patch
4. `arbor-server` exposes live graph over WebSocket
5. `arbor-mcp` wraps graph queries as MCP tools for AI clients
6. `arbor-cli` orchestrates all of the above

### Key Design Decisions

- **Import-aware edge filtering**: Cross-file edges are dropped if the caller's file has explicit imports but does not import the callee's name. Prevents cross-module false positives.
- **Test-file-aware PageRank**: Callers from `test/spec/fixture/mock` files contribute 10% weight, not 100%, to avoid test-inflated centrality scores.
- **Dotted method calls in JS/TS resolve by name only**: `obj.method()` cannot be resolved without the type of `obj`, so `typescript.rs` emits it as the receiver-unknown marker `.method` (`extract_call_references`). The builder links it by method name, **refuses when three or more symbols share that name**, and stamps reduced confidence on what it does link. So the common names — `get`, `find`, `handle`, `execute` — still produce no edge, which is where a SCIP index earns its keep.
- **Stack overflow prevention**: `stacker::maybe_grow` wraps recursive AST traversal in all parsers; `collect_calls` uses iterative `TreeCursor` to avoid deep call stacks.
- **Import nodes excluded from graph**: Import/import-from AST nodes are processed for import map data but never added as vertices (prevents false centrality).
- **Centrality persistence**: `arbor map` computes PageRank on first call and saves it to the binary cache. Subsequent calls skip recomputation (~0.5s vs ~1.5s).
- **Atomic cache writes**: `save_graph_binary`/`save_graph_snapshot` write to `.tmp` then atomically rename, preventing concurrent processes from reading half-written caches.
- **Sled lock avoidance**: CLI commands skip the sled store path if `cache/db` exists (implies a bridge may hold the exclusive lock). Falls back to re-indexing from source.
- **One traversal, four relationship kinds**: `ArborGraph::related` / `related_transitive` / `relationships_by_kind` / `has_edges_of_kind` take a *slice* of `EdgeKind` and a direction, and `implementors` is now a one-line call into them — inheritance is two kinds (`Implements` and `Extends`), so a single-kind signature needs a second implementation on day one. They traverse with `edges_directed`, not `neighbors_directed` + `find_edge` like the older `get_callers`: a SCIP graph really does carry both a `calls` and a `uses_type` edge between the same pair, and `find_edge` returns one edge per pair, so the other kind is invisible. Nodes are deduped, which is why `arbor references` reports 126 symbols where the graph holds 291 edges — a method touching a field five times is one answer, not five.
- **Relationship queries degrade loudly, never silently**: `arbor implementors`/`supertypes`/`uses-type`/`references` and MCP `get_implementors`/`get_supertypes`/`get_type_usages`/`get_references` distinguish three empty results — nothing has that relationship (the graph *does* carry that edge kind), the indexer emits none of that kind (`rust-analyzer` emits no implementation relationships), and the graph is Tree-sitter so the question is unanswerable. Measured, not assumed: Tree-sitter constructs *none* of these kinds — this repo's own Tree-sitter graph is 620 `calls` edges and nothing else. `has_edges_of_kind` is what separates the three, and the JSON carries `hierarchyAvailable`/`usesTypeAvailable`/`referencesAvailable` so a tool cannot re-derive the mistake from a list length. An interface with four implementations reported as "no implementors" is how someone deletes it.
- **SCIP edges bypass name resolution**: `PinnedEdge` endpoints are node IDs, applied after `resolve_edges()`. A compiler-resolved edge routed through name matching would lose the only thing that makes it better than a guess.
- **Indexer knowledge is a table, not a hierarchy**: `arbor-cli/src/indexers.rs` holds one row per indexer — binary name, root markers, arguments, install hint, an optional retry rule, and `also`, extra places the binary may live. Adding a language is one row; a wrong row breaks exactly one project type, and breaks it by printing an install command rather than by guessing. `scip_pipeline.rs` knows only how to run one, retry it if its own rule says so, and collect what it produced.
- **Where an indexer *is* varies as much as what it is called**: `also` entries resolve `~/` against home, anything with a separator against the project root, and a bare name as an alternate on `PATH` — one field, three one-line rules. `dart pub global activate` installs `scip_dart` (underscore) into `~/.pub-cache/bin`, which Pub itself warns is off `PATH`; Composer installs to the project's own `vendor/bin`; `dotnet tool install --global` uses `~/.dotnet/tools`. Before this, Arbor answered a correctly-installed indexer with the very install command the user had just run. `Indexer::resolve_binary` is the single resolver both detection and invocation call, because two code paths that must agree about whether a binary exists will eventually disagree.
- **A polyglot repo runs every indexer that matches**: indexes are collected into `.arbor/scip-indexes/` before ingest, because all of these tools write `index.scip` by default (two indexers would overwrite each other) and because `gradlew clean` deletes `build/`, which would destroy what `.arbor/scip.json` points at. An indexer that fails does not block the others — its language is reported as missing rather than silently absent, since "not indexed" and "nothing calls this" are indistinguishable in a graph. Markers are matched at the repo root only, or `arbor scip` would start a compile for every vendored fixture.
- **SCIP ingestion is language-neutral, and the differences are data**: `SymbolStyle` is a struct of two values, not a trait — the variation between languages is a separator and a constructor name, and expressing that as polymorphism would cost ten times the code. A language Arbor has never seen falls back to a permissive default and still produces a usable graph rather than none.
- **File paths are stripped from SCIP qualified names**: `scip-typescript` puts the source path in the leading namespace descriptors, where `scip-java` puts a package, so a naive join yields `src.userService.ts.UserService.find`. The path is already on `CodeNode.file` and `compute_id` hashes it in, so dropping it costs no uniqueness. Detected by extension (`languages::is_supported`) rather than by indexer, so it covers every path-qualifying indexer without enumerating them.
- **Virtual dispatch expansion**: a call to an interface method also gets edges to its implementations (transitive, depth 4), confidence `1/n` and dropped past 8 candidates. Without it, blast radius on interface-driven JVM code stops at the interface and understates the real reach.
- **SCIP provenance guard**: `arbor scip` writes `.arbor/scip.json`. While it exists, *no* operation lets Tree-sitter replace the graph: reads serve the cache and `load_or_index_graph` refuses its rebuild-and-persist fallthrough (which previously destroyed the graph from a plain read), `index`/`index --changed-only` refuse without `--force`, `status`/`export` report the SCIP graph, and `serve`/`bridge` serve the cache rather than a fresh Tree-sitter index. reads auto-rebuild when sources are newer than `index.scip` (blocking; measured against the index rather than the graph cache, since re-ingesting refreshes the cache without regenerating the index) and never retry a rebuild that already failed for the same sources — `ARBOR_NO_AUTO_REBUILD=1` opts out; `watch` watches without re-parsing and warns once. `gui`/`viz` load the cached graph rather than re-indexing, and `setup` reports an already-SCIP project instead of hitting the `index` guard with no `--force` available. Cache reading and provenance detection live in `arbor-graph/src/cache.rs` so the CLI and GUI share one implementation. Enforced by `refuse_if_scip_provenanced()`. Refreshing needs a full compile: `arbor scip --background` spawns a detached worker (`arbor-cli/src/scip_pipeline.rs` runs whichever indexers the project's markers select, scip-java's config-cache retry being one row's optional rule), records a file-backed handle in `.arbor/scip-task.json` (`arbor-graph/src/scip_task.rs`) that both `arbor scip --task-status` and MCP `tasks/get` can poll, and swaps the graph only on success.
- **Whitespace-only diff filtering**: `git_changed_files()` cross-references `--name-status` against `--numstat` to exclude files with only whitespace changes.

### `.arbor/` Directory

Local cache created by `arbor init`/`arbor setup`. Contains `config.json` with default settings, `graph.bin` (bincode-serialized graph with centrality scores), `graph.json` (JSON snapshot), and — when the graph was built by `arbor scip` — `scip.json` recording which indexes it came from. Treated as workspace root marker alongside `.git`, `Cargo.toml`, `package.json`, `go.mod`, `pyproject.toml`.

## CLI Command Reference

| Command | Purpose |
|---|---|
| `setup .` | One-shot init + index |
| `init .` | Initialize `.arbor/` config |
| `index .` | Parse and build graph |
| `scip index.scip` | Build graph from a SCIP index (compiler-accurate) |
| `map . --exclude-test` | Ranked project skeleton (token-budgeted) |
| `query "name" .` | Fuzzy symbol search (supports `\|` for OR) |
| `callers "sym" .` | Who calls this? |
| `callees "sym" .` | What does this call? |
| `implementors "sym" .` | Who implements/extends this? (alias: `subclasses`; needs a SCIP graph) |
| `supertypes "sym" .` | What does this implement/extend? (needs a SCIP graph) |
| `uses-type "sym" .` | Where does this type appear — field, parameter, return, generic? (needs a SCIP graph) |
| `references "sym" .` | What touches this field, constant, or enum member? (needs a SCIP graph) |
| `entry-points .` | HTTP handlers, main, jobs, webhooks |
| `file-graph "path" .` | Symbols + edges in one file |
| `inspect "sym" .` | Full symbol detail |
| `path "a" "b" .` | Shortest call-graph path |
| `diff .` | Blast radius of git changes |
| `check .` | CI safety threshold check |
| `refactor "sym" .` | Blast radius of changing a symbol |
| `summary .` | Auto-generate PR description |
| `audit "sink" .` | Trace call paths to a sensitive sink |
| `explain "question" .` | Graph-backed context for a question |
| `agent review .` | Autonomous PR risk review |
| `agent onboard .` | Generate contributor onboarding guide |
| `agent guard .` | Architecture violation check |
| `hook claude` | Install hooks into Claude Code settings |
| `doctor .` | System health / environment check |
| `status .` | Index status and statistics |
| `export .` | Export graph to JSON |
| `bridge .` | Start MCP server (stdio; `--http --port 3333` for HTTP) |
| `serve .` | Start WebSocket server |
| `watch .` | File watcher + auto re-index |

All query commands support `--json`. `map` additionally supports `--tokens N`, `--focus "pattern"`, `--focus-changed`, `--verbose`. `uses-type`/`references`/`supertypes` support `--limit N` (default 50, `0` for all) and `--exclude-test`, and always print the pre-limit total — one type in a real Spring codebase has 507 uses, 195 of them in tests, so an unbounded list is the normal case and silent truncation would be a lie.

## Agent Integration (Claude Code)

To integrate arbor into a target project for AI agent use.

**Steps 2–4 are automated:** `arbor hook claude` (add `--global` for the user
config) writes the hooks, the permission allow-list, and the CLAUDE.md guidance
block. It merges rather than overwrites — existing hooks, permissions, `deny`
entries, `env`, and CLAUDE.md content are preserved — and re-running updates the
arbor block in place. It prefers an existing `.claude/CLAUDE.md` if there is one,
otherwise creates `./CLAUDE.md`.

Step 1 is **not** automated: `.mcp.json` must still be written by hand, because
the MCP server entry needs an absolute path to both the `arbor` binary and the
project.

The manual equivalents are documented below so the generated config is
reviewable.

### 1. MCP server (`.mcp.json` at project root)

```json
{
  "mcpServers": {
    "arbor": {
      "command": "/Users/<you>/.cargo/bin/arbor",
      "args": ["bridge", "/absolute/path/to/project"]
    }
  }
}
```

### 2. Hooks (`.claude/settings.json`)

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "echo \"$CLAUDE_TOOL_INPUT\" | grep -q '\\barbor\\b' && [ ! -d .arbor ] && arbor init . >/dev/null 2>&1; exit 0"
          },
          {
            "type": "command",
            "command": "echo \"$CLAUDE_TOOL_INPUT\" | grep -qE '(grep|rg)\\s+(-[a-zA-Z]*r|-[a-zA-Z]*R|--recursive)' && echo 'BLOCK: Use arbor instead of recursive grep.' && exit 1; exit 0"
          }
        ]
      }
    ],
    "PostToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "FLAG=\".arbor/.map-injected-$(date +%Y%m%d)\"; [ -f \"$FLAG\" ] && exit 0; touch \"$FLAG\"; echo '--- arbor map (project skeleton) ---'; arbor map . --exclude-test 2>/dev/null; echo '--- end arbor map ---'; exit 0"
          }
        ]
      }
    ]
  }
}
```

**What these do:**
- **PreToolUse #1**: Auto-initializes `.arbor/` if an arbor command is called but the project isn't set up yet.
- **PreToolUse #2**: Blocks recursive grep/ripgrep and tells the agent to use arbor instead.
- **PostToolUse**: Injects `arbor map` output (project skeleton) on the first Bash call each day. Flag file is per-project (`.arbor/.map-injected-<date>`), so each project triggers independently. Runs with `ARBOR_NO_AUTO_REBUILD=1` — on a SCIP project a stale index would otherwise make this hook block on a full compile, with `2>/dev/null` hiding why.

`arbor hook claude` also allow-lists `arbor scip --task-status` (read-only) but deliberately **not** `arbor scip *`, since that would let an agent start a multi-minute Gradle build unprompted, and its injected guidance tells the agent never to run `arbor index` on a SCIP project.

### 3. Permissions (`.claude/settings.json`)

Written by `arbor hook claude` alongside the hooks above — same file, so team
members get both from one checked-in config.

```json
{
  "permissions": {
    "allow": ["Bash(arbor query *)", "Bash(arbor callers *)", "Bash(arbor map *)", "..."]
  }
}
```

Do **not** use a blanket `Bash(arbor *)`. That allow-lists `arbor scip --background`,
which starts a multi-minute Gradle build, and `arbor index --force`, which
replaces a compiler-resolved graph with guessed edges. The installed list is
read-only commands plus `arbor scip --task-status`.

### 4. Agent instructions (`CLAUDE.md` at project root)

Document the workflow: map is auto-injected, use `arbor query`/`callers`/`callees`/`file-graph` for navigation, only `Read` after arbor identifies the target file+line.

## Sub-Projects Outside the Workspace

- `extensions/arbor-vscode/` — VS Code extension (separate npm project)
- `visualizer/` — Flutter visualizer (separate Dart project, launched by `arbor viz`)
- `packaging/` — Homebrew/Scoop manifests; checksums are pinned per release

## Releasing

Release process is documented in `docs/RELEASING.md`. Releases are driven by tag push through `.github/workflows/release.yml`; separate workflows publish to npm, the VS Code Marketplace, and GHCR. Version lives in `[workspace.package]` in the root `Cargo.toml`.
