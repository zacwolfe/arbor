<p align="center">
  <img src="docs/assets/arbor-logo.svg" alt="Arbor logo" width="120" height="120" />
</p>

<h1 align="center">Arbor</h1>

<p align="center">
  <strong>Graph-native intelligence for codebases.</strong><br>
  Know what breaks <em>before</em> you break it.
</p>

<p align="center">
  <a href="https://github.com/Anandb71/arbor/actions"><img src="https://img.shields.io/github/actions/workflow/status/Anandb71/arbor/rust.yml?style=flat-square&label=Rust%20CI" alt="Rust CI" /></a>
  <a href="https://crates.io/crates/arbor-graph-cli"><img src="https://img.shields.io/crates/v/arbor-graph-cli?style=flat-square&label=crates.io" alt="Crates.io" /></a>
  <a href="https://github.com/Anandb71/arbor/releases"><img src="https://img.shields.io/github/v/release/Anandb71/arbor?style=flat-square&label=release" alt="Latest release" /></a>
  <a href="https://github.com/Anandb71/arbor/pkgs/container/arbor"><img src="https://img.shields.io/badge/GHCR-container-blue?style=flat-square" alt="GHCR" /></a>
  <a href="https://glama.ai/mcp/servers/@Anandb71/arbor"><img src="https://img.shields.io/badge/MCP%20Directory-Glama-6f42c1?style=flat-square" alt="Glama MCP Directory" /></a>
  <img src="https://img.shields.io/badge/license-MIT-green?style=flat-square" alt="MIT License" />
</p>

<p align="center">
  <img src="docs/assets/arbor-demo.gif" alt="Side-by-side: an agent navigating tokio with grep-and-read (47 tool calls, still searching) vs the same agent with arbor's code graph (4 graph calls + 1 read, done)" width="900" />
</p>
<p align="center">
  <sub>Simulated replay — the <code>arbor</code> commands and their output are real (tokio @ 178k LOC). Methodology: <a href="docs/BENCHMARKS.md">BENCHMARKS.md</a></sub>
</p>

> **v3.0.0 — The Right Node** · v2.6.0 stopped *dropping* colliding symbols. It did not stop resolving them to the wrong one. When a bare name matched several modules, resolution fell through to "same directory" and confidently attached the edge to whichever definition happened to sit next to the caller. On a graded fixture the three largest hubs reported **zero downstream impact** while unrelated siblings inherited their centrality. A file's own imports now settle it. Reproduce it yourself: [getArbor-dev/arbor-torture](https://github.com/getArbor-dev/arbor-torture)

---

## Why Arbor

Most AI coding tools treat code as text. Arbor builds a **semantic dependency graph** — functions, classes, and modules as nodes; calls, imports, and inheritance as edges — then answers execution-aware questions with deterministic precision:

| Question | Arbor answer |
|----------|--------------|
| *If I change this symbol, what breaks?* | Blast radius with depth, confidence, and risk level |
| *Who calls this — directly and transitively?* | Caller/callee traversal on the call graph |
| *What's the shortest path between A and B?* | A* path through real dependencies |
| *Is this PR too risky to merge?* | CI gate on blast-radius thresholds |

No keyword guessing. No embedding hallucinations. One graph, every interface.

Where the graph is *unsure*, it says so — edges carry a confidence, and ambiguous resolutions are labelled rather than hidden. An honest unknown beats a confident wrong answer.

---

## What's new in v3.0.0

One fix, measured.

**Symbol resolution consults the importing file.** When a bare name matched
definitions in several modules, `resolve_ref` fell through to `SameDir` and
attached the edge to whichever definition sat in the caller's own directory —
not a dropped edge, a confidently misrouted one, stamped at 0.55 confidence.

`GraphBuilder` already kept a per-file import map, but only
`apply_import_validation` read it, and that scores an edge *after* one has been
chosen. It never saw the references going to the wrong node. Consulting it
between the same-file and same-directory checks keeps a local definition
shadowing an import, while letting a written import beat mere adjacency.
`Resolution::ViaImport` scores 0.93, above `SameDir`'s 0.55.

### Measured

A fixture of 260 modules across 10 layers, each layer defining the same 26
function names. Ground truth is derived from the generator's own edge list, so
the expected answer is exact rather than estimated.

| True downstream | v2.6.0 | v3.0.0 |
|---|---|---|
| 179 | 0 | **163** |
| 178 | 0 | **161** |
| 161 | 0 | **133** |
| 143 | 22 | **133** |
| 122 | 22 | **119** |
| 36 | 22 | 61 |
| 16 | 22 | 46 |

Previously flat at about 22 regardless of the real answer. Now it tracks. Risk
on the largest hub moves from `LOW` to `CRITICAL`.

Total edge count barely moves (1335 → 1334). That is the signature of
misrouting rather than loss: the edges were always there, pointing at the wrong
nodes.

### Breaking

- `Resolution` gains a `ViaImport` variant — an exhaustive match will not compile
- Edges land on different nodes, so cached graphs, stored node ids, and
  centrality baselines from 2.6.0 will differ

### Known and still open

Written down rather than left to be discovered:

- Small targets now **over**-report (36 → 61, 16 → 46). Safer direction than
  silence, but not yet correct.
- PageRank has no escape from a closed cycle. Every member of a 500-function
  ring scores above 90% centrality on one caller each, so mutually recursive
  clusters — parsers, tree walkers, state machines — crowd the top of any
  ranking.
- Inheritance produces no edges. `class Middle(Base)` is invisible, so changing
  a base class shows zero blast radius.
- Dynamic and reflective imports (`importlib`, `__import__`, `import()`,
  `eval(require(...))`) are unresolvable by construction and are documented as
  expected misses in the fixture rather than counted as defects.

<details>
<summary><strong>v2.6.0 — Ground Truth</strong> (colliding symbols kept, deterministic resolution, edge confidence, percentile centrality)</summary>

Correctness, not speed. Each of these was silently wrong before.

| Fix | Why it mattered |
|-----|-----------------|
| **Colliding symbols are kept** | `SymbolTable` used `HashMap::insert`, so a second `handler`, `new`, or `process` replaced the first. The loser had zero callers and was invisible to blast radius. |
| **Resolution is deterministic** | Same-directory locality was decided by iterating a `HashMap`. Rust seeds `RandomState` per process, so the same binary on the same input could build different edges between runs. Now asserted across eight fresh processes. |
| **Edges carry confidence** | A proven same-file call and a same-directory guess were identical evidence. Each edge now scores `[0,1]` by how it resolved. |
| **Exported TS symbols indexed once** | `export_statement` recursed into its children, then the generic loop recursed again — every exported symbol became two vertices sharing one node id. **133 phantom nodes on a 149-file app, 25% of the graph.** |
| **Method calls on untyped receivers resolve** | `obj.method()` was dropped outright, leaving the graph nearly edgeless on TS/JS — and an empty graph reports a blast radius of zero, which reads as "safe" rather than "unknown". |
| **Centrality is a percentile rank** | Scores were divided by the graph maximum, so the top node was `1.0` by construction and a `0.6` threshold meant nothing consistent between repos. Adding one hub rescaled every other node. |
| **Resolution is O(1), not O(refs × nodes × files)** | Unresolvable references — stdlib and third-party calls, most call sites in real code — paid the worst case. Suffixes are now indexed. |

**New capability — concept search.** Substring matching cannot find `get_authenticated` from `login`; they share no substring. Identifiers are now tokenized and expanded through curated concept clusters, and docstrings, signatures, and paths are indexed alongside names. Deterministic, offline, no model. Available on the library as `ArborGraph::search_ranked` (`arbor query` remains literal-substring for now).

**New capability — hunk-level impact.** `changed_node_ids_for_ranges` keeps only symbols whose lines actually changed, instead of every symbol in a touched file.

Measured on identical node sets, after the duplicate-extraction fix:

| Codebase | Before | After |
|----------|--------|-------|
| TypeScript (149 files) | 172 edges | **196** (+14%) |
| Rust (arbor-graph) | 116 edges | **167** (+44%) |

Graph caches from earlier versions are invalidated — centrality now means something different, so a stale cache would be read wrong.

</details>

<details>
<summary><strong>v2.5.0 — The Last Excuse</strong> (PageRank 23x, parallel indexing, warm-start centrality)</summary>

| Change | Measured |
|--------|----------|
| **PageRank rewrite** — flat call-graph adjacency replaces per-iteration traversal | 149.8ms → **6.6ms** on a 10k-node graph (**23x**), verified side-by-side vs the old implementation |
| **Parallel indexing** — parse fans out across all cores, deterministic assembly | Arbor: 253ms → **95ms** · tokio (178k LOC): 2.7s → **1.6s** |
| **Warm-start centrality** — watcher recomputes seed from previous scores | Converges in ~2 rounds after a one-file patch instead of the full 20-iteration budget |
| **Convergence early-exit** | Iteration stops at 1e-9 max delta — the budget is a ceiling, not a sentence |

Think a number is wrong? `cargo bench -p arbor-graph` and prove it: [BENCHMARKS.md](docs/BENCHMARKS.md).

</details>

<details>
<summary><strong>v2.4.0 — The Agent-Native Leap</strong> (MCP <code>2026-07-28</code>, HTTP transport, Tasks, MCP Apps)</summary>

| Feature | What it does |
|---------|--------------|
| **MCP `2026-07-28`** | Stateless `server/discover`, response caching (`ttlMs`/`cacheScope`), dual-version fallback for `2025-03-26` clients |
| **Tasks extension** | `tasks/get` · `tasks/update` · `tasks/cancel` — cold-start indexing returns task handles, not errors |
| **MCP Apps** | Interactive blast-radius graph (`ui://arbor/blast-radius`) and architecture map (`ui://arbor/architecture-map`) inside agent hosts |
| **HTTP transport** | `arbor bridge --http --port 3333` — stateless MCP behind load balancers |
| **Real `get_blast_radius`** | Git-diff-aware impact analysis via shared `arbor-graph::compute_blast_radius` |
| **Pagination** | `offset` / `limit` / `hasMore` on `search_symbols` and `get_map` |
| **Benchmarks** | Criterion suite + CI regression gate — see [BENCHMARKS.md](docs/BENCHMARKS.md) |

</details>

---

## Quickstart

```bash
# Install
cargo install arbor-graph-cli

# Index your project (one command)
cd your-project && arbor setup

# Explore before you edit
arbor map . --exclude-test          # ranked project skeleton (~1k tokens)
arbor refactor parse_file           # blast radius of changing a symbol
arbor diff                          # impact of uncommitted git changes

# Wire up your AI agent
claude mcp add --transport stdio --scope project arbor -- arbor bridge
```

**Agent workflow:** call `get_map` first → `search_symbols` / `get_file_graph` to locate code → `Read` only the target file. [Full MCP guide →](docs/MCP_INTEGRATION.md)

---

## For AI agents (MCP)

Arbor ships a production MCP server via `arbor bridge`. Stdio is the default; HTTP is opt-in for remote/enterprise.

```bash
# Stdio (Claude, Cursor, VS Code)
arbor bridge

# HTTP (MCP 2026-07-28)
arbor bridge --http --port 3333
```

### Cursor / VS Code

```json
{
  "mcpServers": {
    "arbor": {
      "type": "stdio",
      "command": "arbor",
      "args": ["bridge"]
    }
  }
}
```

Templates: [`templates/mcp/`](templates/mcp/) · Setup scripts: `scripts/setup-mcp.sh` · `scripts/setup-mcp.ps1`

### 16 MCP tools

| Tier | Tools | Use when |
|------|-------|----------|
| **Orientation** | `get_map` | First call — token-budgeted project skeleton ranked by PageRank |
| **Surgical** | `list_entry_points` · `get_callers` · `get_callees` · `search_symbols` · `get_file_graph` · `get_node_detail` | Navigate to a specific symbol or file |
| **Broad** | `get_logic_path` · `analyze_impact` · `find_path` · `get_knowledge_path` | Trace dependencies, blast radius, paths |
| **Agent-native** | `get_blast_radius` · `explain_symbol` · `audit_security` · `get_architecture_overview` · `batch_query` | PR impact, onboarding, security audit, bulk lookup |

Every tool returns `{ ok, tool, data, meta: { suggested_next_tool, suggested_next_args } }` so agents chain calls without re-prompting.

**Registry:** `io.github.Anandb71/arbor` · [Official API lookup](https://registry.modelcontextprotocol.io/v0.1/servers?search=io.github.Anandb71/arbor) · [Glama listing](https://glama.ai/mcp/servers/@Anandb71/arbor)

---

## CLI reference

| Command | Description |
|---------|-------------|
| `arbor setup` | One-shot init + index |
| `arbor map` | Ranked, token-budgeted project skeleton |
| `arbor query <term>` | Fuzzy symbol search (supports `\|` OR) |
| `arbor callers / callees <sym>` | One-hop graph traversal |
| `arbor entry-points` | HTTP handlers, main, jobs, webhooks |
| `arbor file-graph <path>` | Symbols + edges in one file |
| `arbor inspect <sym>` | Full symbol detail |
| `arbor path <a> <b>` | Shortest call-graph path |
| `arbor refactor <sym>` | Blast radius before refactoring |
| `arbor diff` | Git-change impact report |
| `arbor check` | CI safety gate (`--max-blast-radius N`) |
| `arbor summary` | Auto-generate PR description |
| `arbor agent review` | Autonomous PR architecture review |
| `arbor agent onboard` | Codebase onboarding guide |
| `arbor agent guard` | Real-time architectural safety gate |
| `arbor scip <index.scip>` | Build the graph from a compiler-produced SCIP index |
| `arbor bridge` | MCP server (add `--http` for HTTP transport) |
| `arbor watch` | Live re-index on file changes |
| `arbor gui` | Native desktop UI |

All query commands support `--json`. `map` additionally supports `--tokens N`, `--focus "pattern"`, `--focus-changed`.

---

## Visual tour

<p align="center">
  <img src="docs/assets/visualizer-screenshot.png" alt="Arbor visualizer screenshot" width="760" />
</p>

Full recording: [media/recording-2026-01-13.mp4](media/recording-2026-01-13.mp4)

---

## Installation

```bash
# Rust / Cargo
cargo install arbor-graph-cli

# Homebrew (macOS/Linux)
brew install Anandb71/tap/arbor

# Scoop (Windows)
scoop bucket add arbor https://github.com/Anandb71/arbor && scoop install arbor

# npm wrapper (cross-platform)
npx @anandb71/arbor-cli

# Docker
docker pull ghcr.io/anandb71/arbor:latest
```

No-Rust installers:

- macOS/Linux: `curl -fsSL https://raw.githubusercontent.com/Anandb71/arbor/main/scripts/install.sh | bash`
- Windows: `irm https://raw.githubusercontent.com/Anandb71/arbor/main/scripts/install.ps1 | iex`

Pinned installs: [docs/INSTALL.md](docs/INSTALL.md)

---

## Language support

**Production parsers:** Rust · TypeScript / JavaScript · Python · Go · Java · C / C++ · C# · Dart

**Fallback parsers:** Kotlin · Swift · Ruby · PHP · Shell

[Adding languages →](docs/ADDING_LANGUAGES.md)

Any language with a SCIP indexer can go further than Tree-sitter allows — see
below.

---

## Compiler-accurate graphs via SCIP

Arbor ingests any [SCIP](https://github.com/scip-code/scip) index, whatever
produced it: `scip-java` (Java, Kotlin), `scip-typescript`, `scip-python`,
`rust-analyzer scip`, `scip-clang`, `scip-dotnet`, `scip-go`, `scip-ruby`,
`scip-php`, `scip-dart`. The format does not vary by producer, so one code path
reads them all — see [docs/SCIP.md](docs/SCIP.md).

The rest of this section walks through `scip-java`, because it is the one Arbor
also **runs** for you (`arbor scip --background`) and the one verified against a
real index. For the others: run the indexer, then
`arbor scip index.scip --root .`.

### Why it matters, in Java

Tree-sitter reads source text. It cannot resolve this:

```java
public void pay() {
    gateway.charge();   // which charge()? depends on the type of `gateway`
}
```

Arbor records that call as the reference `gateway.charge`. No symbol has that
name — `gateway` is a variable, not a type — so the reference does not resolve
and **no edge is created**. Deliberate: inventing an edge to whichever
`charge()` happened to match would be a false dependency. But on
interface-driven JVM code, that is most of the call graph missing.

Answering it needs type inference, which is a compiler's job. So instead of
reimplementing javac, Arbor ingests [SCIP](https://github.com/scip-code/scip)
indexes produced by [`scip-java`](https://github.com/scip-code/scip-java),
which runs as a compiler plugin — its resolution *is* the compiler's.

| | Tree-sitter | SCIP |
|---|---|---|
| `foo()` | resolved by name, confidence-weighted | exact |
| `obj.method()` | **no edge** | exact |
| Overloads | share one node | distinct nodes |
| Interface → impl | not represented | `Implements` edges |
| Virtual dispatch | not represented | synthesised call edges |
| Needs a build | no | **yes** |

Steps 2–4 below are automated by
[`scripts/scip-index.sh`](scripts/scip-index.sh) — worth reading the steps once
regardless, since the failure modes are easier to recognise than to debug.

### 1. Install scip-java

Requires **JDK 17+**. This is a one-time, per-machine step — do it *outside*
your project. Any one of these:

```bash
# A. Docker — nothing installed locally. Run from the project root.
docker run -v "$(pwd):/sources" --env JVM_VERSION=17 \
  ghcr.io/scip-code/scip-java:latest scip-java index

# B. Coursier, no install. Jars are cached after the first run.
#    Run from the project root.
coursier launch org.scip-code:scip-java:0.13.1 -- index

# C. Coursier, standalone binary. Run this ONCE, in a directory on your PATH —
#    `-o` writes the launcher to the current directory, so running it at your
#    project root would leave a ~50MB binary in the repo.
cd ~/.local/bin
coursier bootstrap --standalone -o scip-java \
  org.scip-code:scip-java:0.13.1 --main org.scip_code.scip_java.ScipJava
scip-java --help
```

Option C is worth it if you will re-index regularly; A and B need no setup.
Only `scip-java index` (step 2) runs at the project root.

Use the **`scip-code/scip-java`** fork, not Sourcegraph's original. It emits
*typed* occurrence ranges (SCIP 0.9, proto fields 8–11); Arbor reads the
deprecated `repeated int32 range` too, but only the typed encoding is exercised
against real indexes.

### 2. Generate the index

From the repository root, after a build that compiles cleanly:

```bash
scip-java index                    # auto-detects Gradle or Maven
```

**Gradle and Maven only.** `scip-java` describes itself as a Java and Kotlin
indexer, and `--build-tool` accepts `gradle` with Maven auto-detected. An sbt
project fails with `No build tool detected in workspace`; there is no flag that
helps, and Arbor has no Scala parser to fall back on either.

**Gradle with the configuration cache enabled** (`org.gradle.configuration-cache=true`
in `gradle.properties`) fails with two errors — the real one being
`invocation of 'Task.project' at execution time is unsupported` in
`scipPrintDependencies`, and a misleading
`Cannot get property 'dependenciesOut'` downstream of it. scip-java's plugin is
not configuration-cache safe. Disable it for this invocation only:

```bash
scip-java index -- clean scipPrintDependencies scipCompileAll \
  --no-configuration-cache
```

Args after `--` **replace** the build tool's task list rather than append to it,
so every task must be named or nothing gets indexed.

Where do those task names come from? scip-java prints the build command it runs,
prefixed with `$`. The task list is the tail of that line:

```
$ ./gradlew --no-daemon --init-script /tmp/.../init-script.gradle \
    -Dscip.targetroot=... clean scipPrintDependencies scipCompileAll
                          ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
```

`scipPrintDependencies` and `scipCompileAll` will not appear in `./gradlew tasks`
— scip-java creates them at runtime via that injected init script. Run the bare
command once, copy the tasks from its output, then re-run with your flag.

Multi-module builds emit one `index.scip` per module.

### 3. Ingest

```bash
arbor scip index.scip --root .
```

Pass **every** module's index in one call, or cross-module edges will not
resolve — a symbol defined in module B is only linkable while B's definitions
are in scope. [`scripts/arbor-scip-ingest.sh`](scripts/arbor-scip-ingest.sh)
finds them all and makes that single call:

```bash
scripts/arbor-scip-ingest.sh                        # ingest whatever exists
scripts/arbor-scip-ingest.sh --list                 # show what it would do
scripts/arbor-scip-ingest.sh -- --merge             # pass flags to arbor scip
```

Use it rather than `arbor scip $(find . -name 'index.scip') --root .` — the
naive form word-splits on paths containing spaces, silently indexes nothing
when no index exists, and feeds arbor the zero-byte indexes a failed build
leaves behind.

| Flag | Effect |
|------|--------|
| `--root <path>` | Project root the index's relative paths hang off. SCIP paths are relative, Arbor stores absolute; joining here is what makes `arbor diff` and `arbor file-graph` work on the result. |
| `--merge` | Also Tree-sitter-index the files SCIP does not cover, for polyglot repos. |
| `--no-dispatch` | Skip virtual dispatch expansion — only what the compiler literally recorded. |
| `--json` | Machine-readable stats. |

### 4. Query as usual

The graph is the same shape; the edges are just exact.

```bash
arbor callees "pay" .
arbor callers "charge" .
arbor map . --exclude-test
arbor refactor "PaymentGateway.charge" .
```

Agents pick this up for free through the existing MCP tools — there is no
separate SCIP tool, and none is needed. You run `arbor scip` once; `get_callers`,
`analyze_impact`, and `get_map` then return compiler-resolved edges.

### Steps 2–4 in one command

```bash
scripts/scip-index.sh              # from the project root
```

It runs `scip-java index`, and **only if that fails** reads the task list off
the build command scip-java echoes, checks whether the failure was the Gradle
configuration cache, and retries with `--no-configuration-cache`. On a project
without the cache the first attempt succeeds and no second build is run.

It then collects every `index.scip` produced — passing them all to `arbor scip`
in one call, which is required for cross-module edges to resolve — and ingests.

| Flag | Effect |
|------|--------|
| `--background` | Detach the run, log to `$TMPDIR`, return immediately. A scip-java run is a full compile. |
| `--no-ingest` | Stop after producing the index; print the `arbor scip` command instead of running it. |
| `--root <path>` | Project root to operate in (default: `.`). |
| `--dry-run` | Print the commands without executing them. |
| `-- <flags>` | Passed through to `arbor scip` (e.g. `-- --merge`). |

Ingestion is delegated to `arbor-scip-ingest.sh`, so the collection rules live
in one place. Use that script directly when the index already exists and you do
not want to rebuild.

It deliberately does **not** add `--no-configuration-cache` for failures that
are not cache-related — a compile error stays a compile error rather than being
masked as a scip problem, and the build output is surfaced as-is.

### Virtual dispatch

The part type resolution alone does not buy you. A call to
`PaymentGateway.charge()` lands at runtime in `StripeGateway.charge()`. A graph
that stops at the interface reports a blast radius of one for a change that
reaches every implementation.

SCIP records `is_implementation` relationships, so Arbor walks the override
hierarchy and synthesises the missing call edges:

- **Transitive to depth 4** — `interface → AbstractFoo → FooImpl` is three
  levels and entirely ordinary in Spring code.
- **Confidence `1/n`** for `n` candidates. One implementation is `1.0` — not a
  guess, there is exactly one body it can reach.
- **Dropped past 8 candidates.** A `Runnable` in a large codebase has hundreds;
  "calls one of these" carries no information.
- **The interface edge is kept.** Still true, and dropping it would lose the
  fact that the code was written against the abstraction.

`arbor callees` will therefore show both the interface method and its
implementations. Filter on confidence if you want only the certainties.

### Staleness is not silently repaired

`arbor scip` writes `.arbor/scip.json`. While that marker exists, editing a
source file does **not** trigger an automatic Tree-sitter re-index — the cached
graph is served with a warning instead:

```
⚠ Sources changed since this graph was built from a SCIP index.
  Re-run `arbor scip index.scip` to refresh it; serving the cached graph for now.
```

A silent rebuild would swap compiler-resolved edges for guessed ones, which is
a downgrade nobody asked for. That applies to **every** operation that would
rebuild the graph, not just the stale-cache read:

| Command | On a SCIP project |
|---------|-------------------|
| `arbor callers` / `callees` / `map` / … | If sources are newer than `index.scip`, **rebuild synchronously** (runs the compiler), then answer. Otherwise serve the SCIP graph. If the cache cannot be read, refuse rather than fall back to Tree-sitter. |
| `arbor index` | **Refused.** `arbor index --force` is the deliberate downgrade; it proceeds and clears the marker. |
| `arbor index --changed-only` | **Refused.** The worst case, not the mildest: the two indexers build qualified names differently (`Svc.find` vs `com.pkg.Svc.find`), so a partial re-parse *duplicates* nodes rather than replacing them. |
| `arbor status` / `export` | Report and emit the SCIP graph; `status` names its source. |
| `arbor serve` / `bridge` | Serve the cached SCIP graph instead of a fresh Tree-sitter index — otherwise agents get guessed edges over MCP while the cache holds exact ones. |
| `arbor watch` | Watches without re-parsing; warns once that the graph is stale and names the refresh command. |
| `arbor gui` / `viz` | Load the cached SCIP graph instead of re-indexing with Tree-sitter. |
| `arbor setup` | Reports the project as already set up rather than failing on the `index` guard. |

Refreshing after a code change means re-running the compiler, which is a full
build rather than a re-parse. Detach it and get your shell back:

```bash
arbor scip --background          # spawns scip-java + ingest, returns a handle
arbor scip --task-status         # poll it
arbor scip --task-status --json  # same, for agents
```

```
✓ Rebuild started in the background
  task: scip-1788912357
  pid:  48927
  log:  .arbor/scip-rebuild-1788912357.log
```

The current graph stays queryable for the whole build, and the swap is atomic —
a failed build changes nothing at all, and the task records why:

```
scip-1788912357 ✗ failed  (70% — Rebuild failed; the existing graph was left untouched)
error: scip-java failed; see the log.
```

Only one rebuild runs at a time: two concurrent `scip-java` invocations contend
on the same Gradle project lock and would serialise anyway, at the cost of an
unexplained stall.

Agents poll the same handle through the MCP Tasks extension — `tasks/get` falls
back to the on-disk record, since a detached rebuild lives in a different
process from the bridge.

`scripts/scip-index.sh --background` does the same thing without arbor, for
users who would rather not have arbor spawn builds.

### Dirtiness is detected, and acted on

Staleness is measured against **`index.scip`**, not the graph cache — re-running
`arbor scip <index>` rewrites the cache without regenerating the index, so a
cache-based check would call a stale graph fresh.

When any Java source is newer than the index, the next arbor command rebuilds
before answering:

```
⏳ Sources changed since index.scip was built — rebuilding now (this runs the compiler).
  Prefer not to wait? Ctrl-C, then: arbor scip --background
✓ Graph refreshed: 10596 nodes, 45543 edges
```

A stale answer to a question about code you just changed is worse than a slow
one. Four guards keep that from becoming pathological:

| Situation | Behaviour |
|-----------|-----------|
| Rebuild succeeds | Index is refreshed, so the next command does nothing |
| Rebuild fails | Serves the previous graph and answers anyway; **does not retry** until sources change again, so a project that does not compile cannot start a build on every invocation |
| A `--background` rebuild is already running | Waits for it rather than starting a second — concurrent Gradle runs contend on the same project lock |
| `scip-java` not on PATH | Serves the cache and says how to fix the setup |

Opt out entirely with `ARBOR_NO_AUTO_REBUILD=1` — worth setting in CI and in
scripts, where a read command turning into a multi-minute build is not
acceptable:

```
⚠ Sources are newer than the SCIP index, but auto-rebuild is disabled
  (ARBOR_NO_AUTO_REBUILD) — serving the cached graph.
```

### `arbor watch` on a SCIP project

Watches, but never re-parses. On the first source change it says so once and
names the fix:

```
⚠ Sources changed — the graph is now stale.
  Refresh in the background: arbor scip --background
  Poll a background run:     arbor scip --task-status
```

It clears the warning when a background rebuild completes, so the next edit
warns again. No surprise builds, and no Tree-sitter data presented as the graph.

### Measured

A 406-document Spring service, 31 MB index:

```
✓ Ingested 406 documents from scip-java (java)
  10576 definitions across 402 files
  41526 references resolved, 170148 external (JDK, jars, packages), 2055 unattributed
  26767 references ignored (locals, params, type params), 594 self/recursive, 0 unreadable
  837 implements/override edges, 3091 added by dispatch expansion
✓ Graph: 10576 nodes, 45420 edges (43397 confident)
```

Reference counters are exhaustive by construction —
`resolved + external + ignored + unattributed + self + unreadable` equals every
non-definition occurrence in the index (241,090 above, residual zero). Asserted
in tests, because an unaccounted remainder is indistinguishable from a decoding
bug.

`10576` is lower than the index's raw 28,320 definitions, and that is correct:
17,617 are locals and 124 are type parameters, neither of which belongs in a
call graph.

### Limits, written down

- **Needs a successful build.** No compile, no index. An empty index is
  rejected rather than silently accepted as an empty graph.
- **No incremental refresh.** Re-run the indexer and `arbor scip`. `arbor watch`
  does not update a SCIP graph.
- **Reflection and DI wiring are invisible.** Spring `@Autowired` and anything
  reflective are not calls in the index. Dispatch expansion covers the common
  interface-injection case indirectly; a bean resolved purely by name is not
  represented.
- **Visibility is unknown.** SCIP carries no portable access modifiers, so SCIP
  nodes keep the default rather than a guess.

Full reference: [docs/SCIP.md](docs/SCIP.md)

---

## CI & pull requests

```bash
arbor diff --markdown
arbor check --max-blast-radius 30 --markdown
arbor summary
```

GitHub Action (pre-built binary, ~5s vs ~3–5min compile):

```yaml
name: Arbor Check
on: [pull_request]

jobs:
  arbor:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0

      - uses: getArbor-dev/arbor@v3.0.0
        with:
          command: check . --max-blast-radius 30 --markdown
          comment-on-pr: true
          github-token: ${{ secrets.GITHUB_TOKEN }}
```

---

## Architecture

```
arbor-core (Tree-sitter parsing)
    └── arbor-graph (petgraph + PageRank + impact analysis)
            ├── arbor-cli      — CLI + MCP bridge
            ├── arbor-mcp      — MCP protocol server
            ├── arbor-server   — WebSocket JSON-RPC
            ├── arbor-watcher  — incremental file watcher
            └── arbor-gui      — desktop UI
```

**Docs:** [Quickstart](docs/QUICKSTART.md) · [Architecture](docs/ARCHITECTURE.md) · [Graph schema](docs/GRAPH_SCHEMA.md) · [MCP integration](docs/MCP_INTEGRATION.md) · [Benchmarks](docs/BENCHMARKS.md) · [Roadmap](docs/ROADMAP.md) · [Philosophy](PHILOSOPHY.md)

**Release channels:** GitHub Releases · crates.io · GHCR · npm · VS Code / Open VSX · Homebrew · Scoop — [Releasing guide](docs/RELEASING.md)

---

## Philosophy

1. **Consumer first** — beautiful, intuitive, instantly useful
2. **Accessibility second** — works across ecosystems, runs anywhere
3. **Affordability next** — minimal overhead, from laptops to monoliths

Arbor is **local-first**: no mandatory data exfiltration, offline-capable, open source. [Security policy →](.github/SECURITY.md)

---

## Contributing

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets --all-features
```

[CONTRIBUTING.md](.github/CONTRIBUTING.md) · [Good first issues](docs/GOOD_FIRST_ISSUES.md) · [Code of conduct](.github/CODE_OF_CONDUCT.md)

---

## Contributors

<!-- CONTRIBUTORS:START -->
<p align="center">
    <a href="https://github.com/Anandb71" title="Anandb71" style="text-decoration:none; margin:6px; display:inline-block;">
        <img src="https://avatars.githubusercontent.com/u/169837340?v=4" alt="Anandb71" width="72" height="72" loading="lazy" style="border-radius:50%; border:2px solid #30363d; box-sizing:border-box;" />
  </a>
    <a href="https://github.com/holg" title="holg" style="text-decoration:none; margin:6px; display:inline-block;">
        <img src="https://avatars.githubusercontent.com/u/1383439?v=4" alt="holg" width="72" height="72" loading="lazy" style="border-radius:50%; border:2px solid #30363d; box-sizing:border-box;" />
  </a>
    <a href="https://github.com/cabinlab" title="cabinlab" style="text-decoration:none; margin:6px; display:inline-block;">
        <img src="https://avatars.githubusercontent.com/u/66889299?v=4" alt="cabinlab" width="72" height="72" loading="lazy" style="border-radius:50%; border:2px solid #30363d; box-sizing:border-box;" />
  </a>
    <a href="https://github.com/Karthiksenthilkumar1" title="Karthiksenthilkumar1" style="text-decoration:none; margin:6px; display:inline-block;">
        <img src="https://avatars.githubusercontent.com/u/182195883?v=4" alt="Karthiksenthilkumar1" width="72" height="72" loading="lazy" style="border-radius:50%; border:2px solid #30363d; box-sizing:border-box;" />
  </a>
    <a href="https://github.com/zacwolfe" title="zacwolfe" style="text-decoration:none; margin:6px; display:inline-block;">
        <img src="https://avatars.githubusercontent.com/u/2164736?v=4" alt="zacwolfe" width="72" height="72" loading="lazy" style="border-radius:50%; border:2px solid #30363d; box-sizing:border-box;" />
  </a>
    <a href="https://github.com/sanjayy-j" title="sanjayy-j" style="text-decoration:none; margin:6px; display:inline-block;">
        <img src="https://avatars.githubusercontent.com/u/178475117?v=4" alt="sanjayy-j" width="72" height="72" loading="lazy" style="border-radius:50%; border:2px solid #30363d; box-sizing:border-box;" />
  </a>
    <a href="https://github.com/sathguru07" title="sathguru07" style="text-decoration:none; margin:6px; display:inline-block;">
        <img src="https://avatars.githubusercontent.com/u/182798669?v=4" alt="sathguru07" width="72" height="72" loading="lazy" style="border-radius:50%; border:2px solid #30363d; box-sizing:border-box;" />
  </a>
</p>
<p align="center"><sub><strong>7 contributors</strong> | <a href="https://github.com/Anandb71/arbor/graphs/contributors">View all</a></sub></p>

<!-- CONTRIBUTORS:END -->

---

## License

MIT — see [LICENSE](LICENSE).
