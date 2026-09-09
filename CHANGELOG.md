# Changelog

All notable changes to Arbor will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **SCIP ingestion (`arbor-scip` crate):** consumes [SCIP](https://github.com/scip-code/scip) indexes so the graph comes from the compiler's own symbol resolution. Tree-sitter records `gateway.charge()` as the unresolvable reference `gateway.charge` and creates no edge; on a 406-document Spring service this recovered **418 methods** that previously reported zero callers, taking the graph from 11,300 to 45,543 edges. Driven by `arbor scip <index.scip>`.
- **`arbor scip --background` detects and runs the right indexer.** One table row per indexer (`arbor-cli/src/indexers.rs`): binary name, root markers, arguments, install hint, and an optional retry rule — `scip-java`'s Gradle configuration-cache workaround is the only one, as a function pointer rather than a special case. Ten indexers covered: `scip-java`, `rust-analyzer`, `scip-typescript`, `scip-python`, `scip-go`, `scip-dotnet`, `scip-clang`, `scip-ruby`, `scip-php`, `scip-dart`. Detected but not installed prints that indexer's install command instead of failing silently.
- **`arbor implementors <symbol>`** (alias `subclasses`), and MCP `get_implementors`: the types that implement or extend a symbol, with `--transitive` for the concrete leaves. The `Implements` edges a SCIP index produces were previously write-only — they fed dispatch expansion and nothing could read them, since `get_callers` filters to `Calls` and an implementation does not call what it implements. On a real Spring service this answers with 66 implementors of one pipeline interface.
- **Inheritance queries degrade loudly.** Three empty results are distinguished rather than collapsed into "none found": nothing implements it (the graph *does* carry a hierarchy), the indexer emits no implementation relationships (`rust-analyzer`), and the graph came from Tree-sitter so the question is unanswerable — the last two name the producer and the way to get a real answer, and exit 0, because a missing capability is not a user error. `ArborGraph::has_inheritance_edges` separates them, and both the CLI JSON and the MCP envelope carry `hierarchyAvailable` so a consumer cannot re-derive the mistake from a list length. An interface with four implementations reported as "no implementors" is how someone deletes it.
- **Monorepo submodules are detected.** Markers are matched at the repository root *and* in immediate subdirectories, so a Gradle root with a Next.js front end in `ui/` and Python helpers in `scripts/` finds all three instead of only the Gradle build. Depth one deliberately, skipping `node_modules`, `build`, `target`, `dist`, `vendor`, `examples` and hidden directories — recursing would offer to index every vendored fixture in the tree. A build tool detected at the root owns its own submodules, so Arbor does not start a second Gradle pass over one module. A submodule is only run when its indexer is known to emit **root-relative** paths given a directory (verified for `scip-typescript` and `scip-python`); otherwise the module is reported with instructions rather than indexed into a graph naming files that do not exist.
- **`.python-version` no longer makes a Kotlin repository a Python project.** It is a pyenv *tooling* file, so it now counts only when `.py` files sit beside it. A pyenv-managed script collection is still detected; a monorepo whose only Python is eight helpers in `scripts/` gets that module detected on its own `requirements.txt` instead.
- **Detection covers projects with no manifest.** Python markers now include `setup.cfg`, `Pipfile`, `poetry.lock` and `.python-version`, and when no marker matches anywhere Arbor falls back to the source extensions of the files at the repository root — so a pyenv-managed directory of `.py` scripts indexes instead of reporting that nothing applies. The fallback fires only when nothing else matched, so a Rust project with a `release.py` at the root still runs `rust-analyzer` alone. `scip-clang` is excluded from it, since it cannot run without a compilation database.
- **A polyglot repository indexes both languages into one graph.** A root with `Cargo.toml` and `tsconfig.json` runs `rust-analyzer` and `scip-typescript` and ingests both indexes together. Verified end to end: 9 nodes and 6 edges across `.rs` and `.ts`, against 7 nodes and 2 edges from Tree-sitter on the same tree, with `cart.total()` resolving to a real edge. Indexes are collected into `.arbor/scip-indexes/` first, because every one of these tools writes `index.scip` by default and would otherwise overwrite the previous indexer's output — and because `gradlew clean` deletes `build/`, which would destroy what `.arbor/scip.json` points at. An indexer that fails no longer blocks the others; its language is reported as missing from the graph rather than silently absent.
- **Language-neutral SCIP ingestion.** The SCIP grammar does not vary by producer, so one code path reads `scip-java` (Java, Kotlin), `scip-typescript`, `scip-python`, `rust-analyzer scip`, `scip-clang`, `scip-dotnet`, `scip-go`, `scip-ruby`, `scip-php` and `scip-dart`. `SymbolStyle` carries the only two per-language differences — the scope separator (`.` vs `::`) and the constructor name (`<init>`, `constructor`, `__init__`, `initialize`, `__construct`) — chosen from SCIP's `Document.language` and falling back to the symbol's scheme. An unrecognised language gets a permissive default rather than no graph. Path-shaped namespace descriptors, which `scip-typescript` uses where `scip-java` uses a package, are dropped from qualified names: `src/userService.ts`/`UserService#find().` now reads `UserService.find` rather than `src.userService.ts.UserService.find`, and loses no uniqueness because `CodeNode::compute_id` already hashes the file path. Only `scip-java` is *invoked* by Arbor and only it is verified against a real index; the others are bring-your-own-index.
- **Virtual dispatch expansion:** SCIP `is_implementation` relationships are walked transitively (depth 4) so a call to an interface method also reaches its implementations, confidence-weighted `1/n` and dropped past 8 candidates. Without it, blast radius on interface-driven JVM code stops at the interface — `arbor refactor` reported a 248-caller method as an entry point with nothing calling it.
- **`arbor scip --background`:** detached rebuild that invokes `scip-java`, re-ingests, and swaps the graph atomically only on success. Returns a task handle recorded in `.arbor/scip-task.json`; poll with `arbor scip --task-status` or via MCP `tasks/get`, which now falls back to the on-disk record.
- **`PinnedEdge` / `ArborGraph::add_pinned_edges` / `GraphBuilder::add_pinned_edges`:** edges carrying node IDs rather than names, applied after name resolution so a compiler-resolved edge is never shadowed by a guessed one.
- **`arbor index --force`:** the deliberate downgrade from a SCIP graph back to Tree-sitter.
- **`ARBOR_NO_AUTO_REBUILD`:** opts out of the automatic rebuild. Set it in CI and in hooks.
- **`scripts/scip-index.sh` and `scripts/arbor-scip-ingest.sh`:** generate an index (working around `scip-java`'s configuration-cache incompatibility) and ingest every module's index in one call.

### Changed
- **SCIP graphs are never silently replaced by Tree-sitter.** While `.arbor/scip.json` exists: reads serve the cache, `arbor index` and `index --changed-only` are refused without `--force`, `status`/`export` report the SCIP graph, `serve`/`bridge` serve the cache rather than a fresh Tree-sitter index, and `watch` watches without re-parsing. Previously a plain `arbor callers` whose cache failed to load would rebuild with Tree-sitter **and persist it**, destroying the graph as a side effect of a read.
- **Reads auto-rebuild when sources are newer than `index.scip`** (blocking, since refreshing means running the compiler). Staleness is measured against the index rather than the graph cache, because re-ingesting refreshes the cache without regenerating the index. A rebuild that already failed for the same sources is not retried.
- **`arbor viz` honours SCIP graphs.** Like the GUI, it called `index_directory` unconditionally and handed the visualizer a freshly-parsed Tree-sitter graph. Now uses `graph_for_serving`, and reports node/edge counts rather than files indexed.
- **`arbor setup` is no longer a dead end on a SCIP project.** It called `index`, which correctly refuses, but `setup` exposes no `--force` — so the command failed with no way forward. It now reports the project as already set up and names the refresh command.
- **`arbor bridge` announces what it is doing.** On a SCIP project it said "Starting initial index" while actually loading a cache, and reported "0 files".
- **`arbor gui` honours SCIP graphs.** It previously called `index_directory` on every launch, so on a JVM project it displayed a freshly-parsed Tree-sitter graph — on a 406-document service, 11,300 edges instead of 45,543, with methods that have hundreds of callers shown as uncalled. Cache reading and provenance detection moved to `arbor-graph/src/cache.rs`, shared by both front ends.
- **`arbor hook claude`** now allow-lists `arbor status` and `arbor scip --task-status`, and its injected `arbor map` PostToolUse hook runs with `ARBOR_NO_AUTO_REBUILD=1` — otherwise the agent's first tool call of the day could block on a full Gradle compile.

### Fixed
- **`docs/ARCHITECTURE.md`** claimed the WebSocket server listens on `ws://8080`; the default is `7432`.
- **Documentation claimed `scip-java` supports Scala and sbt.** It describes itself as a Java and Kotlin indexer, `--build-tool` takes `gradle` with Maven auto-detected, and an sbt project fails with `No build tool detected in workspace`. Arbor has no Scala parser either — not even a fallback — so Scala projects have no path at all. Corrected in `README.md`, `docs/SCIP.md`, `docs/ARCHITECTURE.md`, `docs/QUICKSTART.md`, `CLAUDE.md`, `arbor scip --help`, and the CLAUDE.md block that `arbor hook claude` writes into user projects.
- **`CLAUDE.md` claimed dotted method calls in JS/TS are excluded from the graph.** They have not been since 2.6.0: `typescript.rs` emits `obj.method()` as the receiver-unknown marker `.method`, and the builder links it by method name at reduced confidence, refusing at three or more candidates. The practical gap is narrower and differently shaped than the docs said — common names like `get` and `find` still produce no edge, rare ones produce a weak one.
- **An automatic rebuild dumped its JSON stats into the middle of a human command.** `arbor refactor` on a stale SCIP project printed a machine-readable block between its own header and its answer — unreadable as prose, unparseable as JSON. The inline rebuild is now silent; the one-line `✓ Graph refreshed` summary it already printed is the report.
- **Indexers that leave `Document.language` empty reported no language.** `scip-python` is one, which showed as `languages: []` and an empty `()` after the tool name. The language is now derived from the symbol's scheme, so the report reads `scip-python 0.6.6 (Python)` — named from the indexer rather than invented.
- **A qualified name could not be used to disambiguate.** When a name matched several definitions, the note said "pass a qualified name to pick a specific one" and printed them — but the symbol index is keyed on the simple name, so every name it printed came back `Symbol not found`. Qualified names now resolve, exactly or by suffix at a scope boundary, so `dedupe_edges.Versions.resolve_edge` and `Prompt.resolve_edge` both work. `.`, `::`, `#` and `/` all count as boundaries, so `Edge` still does not match `ResolvedEdge`.
- **An ambiguous name with no connections picked a field over the function.** Ranking is by degree then centrality, which decide nothing when every candidate is unconnected — so `arbor refactor resolve_edge` analysed a TypedDict annotation rather than the `def` of the same name. Callability now breaks that tie, after degree and centrality, so a well-connected field still outranks an unconnected function.
- **SCIP reported every top-level function as a method.** SCIP spells a free function and a method identically (`()`); what separates them is whether the enclosing scope is a type. Module-level Python, Go and Rust functions were all `method`. The check skips type-parameter descriptors, so `rust-analyzer`'s `impl#[ArborGraph]method()` is still correctly a method.
- **Auto-rebuild messages said "Java sources changed"** on a check that was never Java-specific — `sources_newer_than` tests every supported extension.

## [3.0.0] - 2026-08-10

### Changed
- **Symbols resolve by module, not by nearest directory.** Edges land on different nodes, so cached graphs, stored node IDs, and centrality baselines from 2.6.0 differ.
- `Resolution` gains a `ViaImport` variant — an exhaustive match will not compile.

### Known
- Small targets now over-report blast radius; PageRank has no escape from a closed cycle; inheritance produces no edges; dynamic/reflective imports are unresolvable by construction. See the README for detail.

## [2.6.0] - 2026-08-03 "Ground Truth"

### Fixed
- **Colliding symbols are kept** — `SymbolTable` used `HashMap::insert`, so a second `handler`/`new`/`process` replaced the first and was invisible to blast radius.
- **Resolution is deterministic** — same-directory locality was decided by `HashMap` iteration order, so the same binary on the same input could build different edges between runs.
- **Exported TS symbols indexed once** — `export_statement` recursion created two vertices sharing one node ID (133 phantom nodes on a 149-file app).
- **Centrality is a percentile rank**, comparable across repositories, instead of being divided by the graph maximum.
- **Resolution is O(1)**, not O(refs x nodes x files).

### Added
- **Edge confidence** in `[0,1]`, scored by how the reference resolved.
- **Concept search** (`ArborGraph::search_ranked`) — identifier tokenization plus curated concept clusters, deterministic and offline.
- **Hunk-level impact** (`changed_node_ids_for_ranges`).

## [2.5.0] - 2026-07-15

### Added
- **Parallel indexing:** `index_directory` fans the cache-check/parse phase out across all cores with rayon; results assemble in walk order so graph construction stays deterministic. Measured (median of 3, warm FS cache): Arbor itself 253ms → 95ms (2.7x, 123 files); tokio 2.7s → 1.6s (1.7x, 815 files / 178k LOC — serial graph assembly caps the gain, see `docs/BENCHMARKS.md`). Thread count is tunable via `RAYON_NUM_THREADS`.
- **Warm-start PageRank:** `compute_centrality_warm` seeds iteration from previous scores (with analytic rescaling of the max-normalized stored values back to fixed-point scale) — watcher/server graph patches now converge in a couple of rounds instead of the full iteration budget. Wired into the sync server's re-index and delete paths.
- **Convergence early-exit:** centrality iteration stops once no score moves more than 1e-9 between rounds.
- **Benchmarks:** `compute_centrality_10k` and `compute_centrality_10k_warm` on a realistic fan-in graph (~10k nodes).

### Changed
- **23x faster PageRank:** `compute_centrality` rewritten from per-iteration `get_callers`/string-ID lookups to a one-pass flat adjacency build plus dense Vec iteration — 149.8ms → 6.6ms on a 10k-node graph. Semantics preserved (Calls-edges only, 10% test-caller weight, [0,1] max-normalization).

## [2.4.0] - 2026-07-08 "The Agent-Native Leap"

### Added
- **MCP 2026-07-28 Protocol:** Stateless core with `server/discover`, `_meta` parsing, response caching (`ttlMs`/`cacheScope`), and dual-version fallback for `2025-03-26` clients
- **Tasks Extension:** `tasks/get`, `tasks/update`, `tasks/cancel` — long-running index/audit operations return task handles; fixes cold-start race during background indexing
- **MCP Apps (SEP-1865):** Interactive blast-radius graph (`ui://arbor/blast-radius`) and architecture map (`ui://arbor/architecture-map`) HTML templates rendered inside agent hosts
- **Streamable HTTP Transport:** `arbor bridge --http [--port 3333]` — stateless MCP over HTTP with `Mcp-Method`/`Mcp-Name` header routing, alongside stdio
- **Real `get_blast_radius`:** Git-diff-aware blast radius via shared `arbor-graph::compute_blast_radius` (replaces stub)
- **Pagination:** `offset`/`limit`/`hasMore` on `search_symbols` and `get_map`
- **Criterion Benchmarks:** `cargo bench -p arbor-graph` with CI workflow (`benchmarks.yml`)
- **Release docs:** `docs/ROADMAP_v2.4.0.md`, `docs/RELEASE_NOTES_v2.4.0.md`

### Changed
- **Async MCP stdio:** Replaced blocking `stdin.lock().lines()` with tokio async I/O
- **MCP tool annotations:** `analyze_impact` and `get_architecture_overview` declare `_meta.ui` for MCP Apps
- Workspace version bumped to **2.4.0** across all manifests

## [2.3.0] - 2026-06-28 "Agent Brain"

### Added
- **5 New MCP Tools (10 → 15 total):**
  - `get_blast_radius`: Diff-based impact analysis exposed to AI agents via MCP — returns affected nodes, risk level, and architectural impact
  - `explain_symbol`: Token-bounded architectural explanation of any symbol — role classification, centrality, callers/callees, significance
  - `audit_security`: Traces execution paths from source to sensitive sinks (DB, exec, file I/O, network) — security audit via MCP
  - `get_architecture_overview`: High-level codebase orientation — hotspots, modules, entry points, graph statistics — ideal for onboarding agents
  - `batch_query`: Multi-symbol query in a single call — reduces round-trips for bulk lookups, optional caller/callee inclusion
- **MCP Resources:** Implemented `resources/list` and `resources/read` exposing `arbor://graph/stats`, `arbor://graph/entry-points`, and `arbor://graph/hotspots` for passive agent context
- **MCP Tool Annotations:** All 15 tools annotated with `readOnlyHint`, `destructiveHint`, `idempotentHint`, and `openWorldHint` per 2025-03-26 spec — signals trust/safety to agents
- **Built-in Agent Workflows (`arbor agent`):**
  - `arbor agent review`: Autonomous PR review — analyzes git changes for high-centrality modifications, untested paths, and architecture violations
  - `arbor agent onboard`: Codebase onboarding guide generator — entry points, hotspots, module map, suggested reading order
  - `arbor agent guard`: Architecture guard — validates changes against blast radius thresholds, flags entry point modifications
- **A2A Agent Card:** `agent-card.json` for Agent-to-Agent protocol discovery — enables other agents to find and delegate to Arbor
- **GitHub Action Pre-Built Binary:** Downloads pre-compiled binary from GitHub Releases instead of compiling from source (~5s vs ~3-5min CI step)
- **Benchmarks Document:** Performance claims with reproduction methodology
- **Launch Plan:** Structured go-to-market strategy for organic growth

### Changed
- **MCP Protocol Version:** Bumped from `2024-11-05` → `2025-03-26`
- Workspace version bumped to **2.3.0** across all manifests

## [2.2.0] - 2026-05-30 "PR Intelligence & Sponsorships"

### Added
- **arbor diff --markdown**: Native markdown formatting option for impact analysis reports. Perfect for PR comments, presenting a color-coded risk assessment, changed files list, direct/indirect caller metrics, affected API entrypoints, and actionable suggestions.
- **arbor check --markdown**: Markdown safety check output. Validates change impact against maximum blast radius thresholds and prints color-coded PASS/FAIL status.
- **arbor summary**: Auto-generates structured markdown Pull Request descriptions based on graph diff analysis, classifying changes, mapping scope areas, analyzing blast radius, and automatically recommending relevant reviewers.
- **Upgraded GitHub Action composite steps**: Updated `action.yml` with a new `comment-on-pr` parameter. When running in a PR workflow, it automatically executes the impact analysis, posts a markdown comment, and deduplicates comments by editing previous reports.
- **GitHub Native Sponsorships**: Configured standard `.github/FUNDING.yml` to support GitHub Sponsors, Ko-fi, and custom Stripe billing channels.

### Changed
- Aligned workspace and all package manager manifests to **v2.2.0** (Cargo, npm wrapper, VS Code extension, Scoop, Homebrew).
- Simplified `.github/workflows/arbor-pr-bot.yml` to reference the local upgraded composite action directly, reducing boilerplate logic.
- Ignored `.claude/` locally via `.gitignore`.

## [2.1.0] - 2026-05-15 "Agent-Native MCP Ecosystem Expansion"

### Added
- **MCP Server Upgrade**: Re-engineered model context protocol server to expose **10+ advanced agent-native tools** directly to AI clients.
- Added comprehensive tool schema declarations for AI engine integration.

## [2.0.1] - 2026-04-20 "Patch Stability & Automation Fixes"

### Fixed
- **PR Bot reliability**: switched to valid CLI-driven impact report generation (`arbor diff . --json`) and added PR base/head commit-range support via `ARBOR_DIFF_BASE` / `ARBOR_DIFF_HEAD`.
- **PR Bot formatting**: corrected Markdown code-fence rendering so JSON output appears as a proper fenced block in PR comments.
- **CLI regression coverage**: added integration test coverage for ranged diff mode to prevent regressions in PR-impact reporting.
- **Contributors automation**: hardened `contributors.yml` by skipping PR creation when README has no contributor changes and using `github.token` consistently.
- **Contributors script resilience**: updated GitHub auth header usage and graceful API failure handling to avoid flaky scheduled workflow failures.

### Changed
- Release-facing versions aligned to **2.0.1** across workspace/package-manager/editor manifests (Cargo workspace version, Homebrew, Scoop, npm wrapper, VS Code extension metadata).

## [2.0.0] - 2026-04-20 "Context-Driven OS + Stability Release"

### Added
- **MCP Tool Expansion**: `get_knowledge_path` returns real logic paths with Markdown [[links]] + causality explanations (Aha! for Lattice users). `analyze_impact` supports `format=markdown` for professional PR bot tables with **bold high-risk** files via ConfidenceExplanation + centrality (sorted_by_centrality).
- **Tauri Lattice Companion**: Desktop shell with system tray (Personal OS feel), graph/MCP integration (left stable for later iteration).
- **Parser v2 Registry**: Clean compile_queries helper, Dart fixes (class_definition etc.), Markdown fallback with NodeKind::Section (no dep conflicts).
- **Sled GraphStore**: Incremental persistence, mtime/versioning, centrality precompute comment (Priority 2).
- **PR Bot**: Enhanced action.yml + workflow for blast radius comments using MCP output.

### Changed
- All discussed features stabilized: parser eat-own-dog-food, MCP supercharged for agents (Priority 3), Markdown support, tests 58/58 passing with feedback loop, no mistakes.
- ROADMAP, PHILOSOPHY aligned (Consumer First = stable, Accessibility = registry, Affordability = sled).
- Versions bumped, docs updated, sequential commits on audit-and-testing-overhaul.
- Release automation hardened: fixed contributors workflow failures (tokened GitHub API + robust LF/CRLF marker replacement), fixed aarch64 Linux linker in release cross-compilation, replaced PR bot action mock output with real command execution.
- Distribution manifests fully aligned to 2.0.0 (Homebrew, Scoop, npm wrapper, VS Code extension metadata/lockfile, server serialization version fixture).

### Stable for v2.0 Release
- `v2.0.0` tagged and `v2.0` PR branch prepared; release workflows (release, GHCR, Marketplace, MCP notes) are aligned and stable.

## [Unreleased]

### Added

- None yet.

### Changed

- None yet.

## [1.7.0] - 2026-03-25 "Distribution & Reach"

> **Feature release focused on making Arbor available everywhere — every package manager, every editor, every CI pipeline.**

### Added

- **Automated release workflow** (`release.yml`) — Cross-platform binary builds (5 targets), crates.io publishing, and GitHub Release creation on tag push
- **Homebrew formula** (`packaging/homebrew/arbor.rb`) — macOS/Linux install via `brew install`
- **Scoop manifest** (`packaging/scoop/arbor.json`) — Windows install via `scoop install`
- **npm wrapper** (`packaging/npm/`) — Cross-platform install via `npx @arbor-graph/cli`
- **VS Code extension: 5 new commands** — `arbor.refactor`, `arbor.status`, `arbor.quickPick`, `arbor.diff`, `arbor.index`
- **VS Code extension: Quick-pick command menu** — `Ctrl+Shift+R` for all Arbor actions
- **VS Code extension: Walkthrough onboarding** — Get Started guide with step-by-step setup
- **VS Code extension: New settings** — `arbor.autoIndex`, `arbor.maxBlastRadius`
- **GitHub Sponsors** (`.github/FUNDING.yml`)
- **Academic citation** (`CITATION.cff`)
- **Docker Compose bridge service** for MCP container usage

### Changed

- **Dockerfile** updated to Rust 1.85 with OCI labels, git support
- **docker-compose.yml** modernized (removed deprecated `version` key)
- **VS Code extension categories** improved for marketplace discoverability
- **README badges** expanded (crates.io, GitHub Release, GHCR, Docker)
- **Install instructions** expanded with Homebrew, Scoop, npm, Docker options

### Removed

- **`vscode-publish.yml`** workflow (duplicate of `vscode-marketplace.yml`, caused CI failures)
- Stale `package-lock.json`, `crates/Cargo.lock`, `crates/test_output.txt`

## [1.6.2] - 2026-03-24 "Revival Release: Language Expansion + Developer Momentum"

> **Feature release focused on expanding parser reach, improving live sync coverage, and strengthening release momentum workflows.**

### Added

- **Fallback parser engine** in `arbor-core` for rapid support of additional ecosystems when a full Tree-sitter path is unavailable in all runtime surfaces
- **New language extension support (5+)** via fallback parsing:
  - Kotlin (`.kt`, `.kts`)
  - Swift (`.swift`)
  - Ruby (`.rb`)
  - PHP (`.php`, `.phtml`)
  - Shell (`.sh`, `.bash`, `.zsh`)
- **Regression tests** for fallback parsing in both legacy parser path and query parser v2 path

### Changed

- **Indexer support matrix** now includes fallback-language extensions in support checks
- **Bridge + visualizer sync watchers** now watch and re-index the newly added language extensions
- **CLI empty-graph hints** now include the expanded extension set
- **Workspace crate line bumped** to `1.6.2` and internal crate dependency versions aligned

### Documentation

- Updated release/status messaging and supported-language listings
- Added release notes for `v1.6.2`

## [1.6.1.1] - 2026-03-18 "Maintenance + Ecosystem Alignment"

> **Maintenance release focused on workflow reliability, MCP guidance, and ecosystem currency as of March 18, 2026.**

### Added

- **CLI: `arbor diff`** — Git-aware blast radius preview for changed files
  - Handles rename-aware changed-file detection
  - Ignores whitespace-only diffs
  - Filters generated/internal files for cleaner signal
- **CLI: `arbor check`** — CI-oriented risk gate over changed blast radius
  - Supports machine-readable JSON output for automation
- **CLI: `arbor open <symbol>`** — Opens symbol/file location in configured editor
- **CLI: `arbor index --changed-only`** — Incremental re-index path based on git changes
- **Binary graph snapshots** — `.arbor/graph.bin` read/write support for faster warm starts
- **Integration tests for diff heuristics** — rename, whitespace-only, generated-file scenarios
- **Workspace cleanup scripts** — `scripts/clean.ps1` and `scripts/clean.sh` to safely prune large generated artifacts before releases

### Changed

- **Branching guidance** documented for `main`, `release/v1.5`, and `release/v1.6`
- **Documentation refresh** across README, Quickstart, Install, Architecture, and MCP integration guides
- **Troubleshooting guidance** now includes a dedicated workflow for reclaiming multi-GB workspace bloat

### Maintenance

- Release channel and status messaging aligned to the `1.6.1.1` maintenance cut.
- Workspace crate version advanced to `1.6.1` (SemVer-compliant crate line for Cargo).
- MCP server metadata version now reports `1.6.1.1` for client-visible maintenance tracking.
- Release context refreshed against current ecosystem signals (Rust `1.94.0`, tree-sitter `0.26.7`, and broader MCP client/platform adoption).

### Documentation

- Added formal release notes for v1.6.0 in `docs/RELEASE_NOTES_v1.6.0.md`
- Added formal release notes for v1.6.1.1 in `docs/RELEASE_NOTES_v1.6.1.1.md`

## [1.6.0] - 2026-03-16

> Release notes for this version are no longer published.

## [1.5.0] - 2026-02-xx

> Maintenance release. See git history for details.

## [1.4.0] - 2026-02-xx "The Trust Update"

> Release notes for this version are no longer published.

## [1.3.0] - 2026-01-xx

> Stabilization and UX improvements. See git history for details.

## [1.2.0] - 2026-01-xx

> Incremental improvements. See git history for details.

## [1.1.0] - 2026-01-08 "The Sentinel Update"

> **Predict breakage. Give AI only the logic it needs.**

### Added

- **Impact Radius Simulator** (`impact.rs`) — Bidirectional BFS to predict all affected nodes before refactoring
  - Severity classification: direct (1 hop), transitive (2-3), distant (4+)
  - Entry edge tracking for explainability
  - Stable ordering for reproducible output
  - 8 unit tests including cycle detection
- **Dynamic Context Slicing** (`slice.rs`) — Token-bounded context extraction for LLM prompts
  - Pinning support for critical nodes
  - Explicit truncation reasons (budget vs depth)
  - 6 unit tests
- **MCP `analyze_impact` Tool** — Structured JSON output for AI agents
  - Input: `{ "node_id": "...", "max_depth": 5 }`
  - Returns: target, upstream, downstream, severity, hop_distance, entry_edge
- **CLI: `arbor refactor <target>`** — Preview blast radius before making changes
  - `--why` flag shows reasoning for each affected node
  - `--json` flag for scripting and CI integration
  - `--depth N` controls search depth
- **CLI: `arbor explain <target>`** — Graph-backed context for code explanations
  - `--why` flag shows path traced
  - `--json` flag for structured output
  - `--tokens N` controls context budget

### Changed

- MCP `analyze_impact` now uses real graph traversal (was placeholder)

## [1.0.0] - 2026-01-07

### Added

- **World Edges (Cross-File Resolution)** - Implemented `SymbolTable` and FQN-based linking for robust cross-file references.
- **Persistence Layer** - Integrated `sled` database for local graph storage (`GraphStore`).
- **ArborQL (MCP)** - Added `find_path` tool for finding shortest paths between nodes.
- **C# language support** - Methods, classes, interfaces, structs, constructors, properties
- **Control Flow edges** - `FlowsTo` edge kind for CFG (Control Flow Graph) analysis
- **Data Flow edges** - `DataDependency` edge kind for DFA (Data Flow Analysis)
- **Barnes-Hut QuadTree** - O(n log n) force simulation for visualizer scalability
- **Viewport culling** - Only render visible nodes/edges for 100k+ node support
- **LOD rendering** - Simplified node rendering at low zoom levels
- **Headless mode** - `--headless` CLI flag for remote/Docker/WSL deployment
- **Binary serialization** - `bincode` dependency for future binary wire protocol

### Changed

- Consolidated language parsers into query-based `parser_v2.rs`
- Upgraded supported languages to 10 (TypeScript, JavaScript, Rust, Python, Go, Java, C, C++, Dart, C#)
- Improved graph rendering performance for large codebases

### Fixed

- None

## [0.1.1] - 2026-01-06

### Added

- **Go language support** - Functions, methods, structs, interfaces, imports
- **Java language support** - Classes, interfaces, methods, constructors, fields
- **C language support** - Functions, structs, enums, typedefs, includes
- **C++ language support** - Classes, namespaces, structs, functions, templates
- **Dart language support** - Classes, mixins, extensions, methods, enums
- `Constructor` and `Field` node kinds for Java/OOP languages
- Updated set-topics workflow with 19 repository topics

### Changed

- Expanded supported languages from 4 to 9
- Updated README with new language support table

### Fixed

- None

## [0.1.0] - 2026-01-05

### Added

- Initial release
- Core AST parsing with tree-sitter
- TypeScript/JavaScript language support
- Rust language support
- Python language support
- Interactive force-directed graph visualizer (Flutter)
- WebSocket-based real-time updates
- MCP (Model Context Protocol) bridge for AI agents
- CLI with `parse`, `graph`, and `bridge` commands
- File watching with hot reload
