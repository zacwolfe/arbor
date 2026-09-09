<p align="center">
  <img src="https://raw.githubusercontent.com/Anandb71/arbor/main/docs/assets/arbor-logo.svg" alt="Arbor" width="80" height="80" />
</p>

<h1 align="center">arbor-graph-cli</h1>

<p align="center">
  <strong>The command-line interface for Arbor</strong><br>
  <em>Index your code. Query the graph. Navigate with AI.</em>
</p>

<p align="center">
  <a href="https://crates.io/crates/arbor-graph-cli"><img src="https://img.shields.io/crates/v/arbor-graph-cli?style=flat-square&color=blue" alt="Crates.io" /></a>
  <a href="https://github.com/Anandb71/arbor"><img src="https://img.shields.io/badge/repo-arbor-green?style=flat-square" alt="Repo" /></a>
  <img src="https://img.shields.io/badge/license-MIT-green?style=flat-square" alt="License" />
</p>

---

## What is Arbor?

Arbor is the **graph-native intelligence layer for code**. It parses your codebase into an AST graph where every function, class, and variable is a node, and every call, import, and inheritance is an edge.

This CLI is the primary interface for indexing, querying, and connecting your code to AI via the Model Context Protocol (MCP).

> Release status (July 2026): **v2.4.0** — MCP `2026-07-28`, Tasks extension, MCP Apps, and `arbor bridge --http`.

## Installation

```bash
cargo install arbor-graph-cli
```

## Quick Start

```bash
# One-shot setup in your project
cd your-project
arbor setup

# Run health diagnostics
arbor doctor

# Start the AI bridge + visualizer
arbor bridge --viz
```

## Commands

| Command | Description |
|---------|-------------|
| `arbor setup` | One-shot setup (init + index) |
| `arbor init` | Creates `.arbor/` config directory |
| `arbor index` | Full index of the codebase |
| `arbor index --changed-only` | Incremental index of git-modified files |
| `arbor index --force` | Re-index with Tree-sitter even on a SCIP project (downgrades the graph) |
| `arbor scip <index.scip>` | Build the graph from a compiler-produced SCIP index (JVM) |
| `arbor scip --background` | Regenerate the index and re-ingest in a detached process |
| `arbor scip --task-status` | Poll a detached rebuild |
| `arbor map` | Ranked, token-budgeted project skeleton |
| `arbor callers <symbol>` | Who calls this? (one hop upstream) |
| `arbor callees <symbol>` | What does this call? (one hop downstream) |
| `arbor entry-points` | HTTP handlers, main functions, webhooks, jobs |
| `arbor file-graph <path>` | Symbols + internal edges within one file |
| `arbor inspect <symbol>` | Full detail on one symbol |
| `arbor path <a> <b>` | Shortest call-graph path between two symbols |
| `arbor summary` | Auto-generate a pull request description |
| `arbor agent review` | Autonomous PR architecture review |
| `arbor agent onboard` | Generate a contributor onboarding guide |
| `arbor agent guard` | Architecture violation check |
| `arbor hook claude` | Install Arbor directives + hooks into Claude Code |
| `arbor query <q>` | Search the graph |
| `arbor diff` | Preview blast radius for current git changes |
| `arbor check` | CI safety gate for risky change sets |
| `arbor open <symbol>` | Open symbol/file in your editor |
| `arbor refactor <symbol>` | Blast-radius preview before refactoring |
| `arbor explain <symbol>` | Graph-backed context for code explanation |
| `arbor audit <sink>` | Security path tracing to sensitive sinks |
| `arbor serve` | Start the WebSocket server |
| `arbor export` | Export graph to JSON |
| `arbor status` | Show index statistics |
| `arbor watch` | Continuous re-index on file changes |
| `arbor bridge` | Start MCP server for AI integration |
| `arbor bridge --http` | MCP over stateless HTTP (port 3333) |
| `arbor bridge --viz` | MCP + Visualizer together |
| `arbor viz` | Launch the Logic Forest visualizer |
| `arbor gui` | Launch native Arbor GUI |
| `arbor pr-summary` | Generate impact summary for pull requests |
| `arbor doctor` (`check-health`) | System diagnostics |

## CI and Team Use

```bash
# Incremental refresh
arbor index --changed-only

# Pull-request safety checks
arbor diff
arbor check --json --max-blast-radius 30
```

## Release Docs

- [Quickstart](../../docs/QUICKSTART.md)
- [Installation](../../docs/INSTALL.md)
- [MCP Integration](../../docs/MCP_INTEGRATION.md)
- [Changelog](../../CHANGELOG.md) — full release history
- [SCIP ingestion](../../docs/SCIP.md) — compiler-accurate graphs for JVM projects

## Supported Languages

Rust, TypeScript, JavaScript, Python, Go, Java, C, C++, C#, Dart, Kotlin, Swift, Ruby, PHP, Shell

## Links

- **Main Repository**: [github.com/Anandb71/arbor](https://github.com/Anandb71/arbor)
- **Documentation**: [docs/](https://github.com/Anandb71/arbor/tree/main/docs)
- **Glama MCP Directory**: [glama.ai/mcp/servers/@Anandb71/arbor](https://glama.ai/mcp/servers/@Anandb71/arbor)

## Environment variables

| Variable | Effect |
|----------|--------|
| `ARBOR_AUTO_INDEX=1` | Allow indexing a project that has no `.arbor/` yet |
| `ARBOR_NO_AUTO_REBUILD=1` | On a SCIP project, never rebuild automatically when sources are newer than the index. Set this in CI and in hooks — otherwise a read command can block on a full compile. |
| `ARBOR_DIFF_BASE` / `ARBOR_DIFF_HEAD` | Override the git range used by `diff`/`check` |
| `ARBOR_EDITOR` | Editor used by `arbor open` |

## SCIP projects (JVM)

When a graph was built by `arbor scip`, `.arbor/scip.json` records it and Arbor
refuses to let Tree-sitter replace that graph: `arbor index` errors without
`--force`, reads serve the cache, and `serve`/`bridge` serve the cache rather
than a fresh Tree-sitter index. See
[docs/SCIP.md](https://github.com/Anandb71/arbor/blob/main/docs/SCIP.md).
