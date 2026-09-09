# SCIP Ingestion — Compiler-Accurate Graphs

Arbor's own parsers read source text with Tree-sitter. That is fast and needs
no build, but it cannot resolve this:

```java
public void pay() {
    gateway.charge();   // which charge()? depends on the type of `gateway`
}
```

Arbor records that call as the reference `gateway.charge`. No symbol has that
name — `gateway` is a variable, not a type — so it does not resolve and **no
edge is created**. That is deliberate: inventing an edge to whichever
`charge()` happened to match would be a false dependency. But on
interface-driven JVM code, that is most of the call graph missing.

Answering it properly requires type inference, which is a compiler's job, not a
grammar's.

Rather than reimplement a type checker per language, Arbor consumes [SCIP], the
open code indexing format. Its indexers run as compiler plugins or on top of a
language server, so their symbol resolution *is* the compiler's. Arbor keeps the
layers it is genuinely good at — ranking, entry-point detection, context
slicing, MCP — and stops guessing at the part a compiler already knows.

## Which indexers work

Ingestion is language-neutral: the SCIP grammar is identical whatever produced
the index, so one code path reads them all. Only two things vary per language —
how scopes are spelled (`.` versus `::`) and what an indexer calls a constructor
— and both live in `SymbolStyle` in `crates/arbor-scip/src/symbols.rs`.

| Language | Indexer | Status in Arbor |
|---|---|---|
| Java, Kotlin | [`scip-java`] | **verified** on a 406-document Spring service. The only one Arbor also *runs* for you |
| Rust | [`rust-analyzer`] | **verified** on Arbor itself — 69 documents, 2,109 nodes, 10,554 edges |
| TypeScript, JavaScript | [`scip-typescript`] | ingests; bring your own index |
| Python | [`scip-python`] | ingests; bring your own index |
| C, C++ | [`scip-clang`] | ingests; bring your own index |
| C# | [`scip-dotnet`] | ingests; bring your own index |
| Go | [`scip-go`] | ingests; bring your own index |
| Ruby | [`scip-ruby`] | ingests; bring your own index |
| PHP | [`scip-php`] | ingests; bring your own index |
| Dart | [`scip-dart`] | ingests; bring your own index |

"Bring your own index" means the ingestion path is shared and exercised by unit
tests, but no one has run that indexer end to end on a real repository and
checked the result. Expect it to work; report it if it does not.

Only `scip-java` is *invoked* by Arbor, because only its build-tool quirks have
been worked through — see `crates/arbor-cli/src/scip_pipeline.rs`. Everything
else you run yourself, then hand the `index.scip` to `arbor scip`.

`scip-java` upstream describes itself as a Java and Kotlin indexer. **Scala is
not supported** by it, and Arbor has no Scala parser either — not even a
fallback — so Scala projects have no path today.

## Integrating an indexer

Every one of these follows the same three steps: install the indexer, produce
`index.scip`, ingest it. Only step 2 differs.

```bash
arbor scip index.scip --root .     # step 3, identical in every case
```

Commands below are from each indexer's own documentation. Versions move; if one
disagrees with reality, its README wins — the link is in the heading.

### Java, Kotlin — [`scip-java`]

Needs **JDK 17+** and a build that compiles. Gradle and Maven only.

```bash
# Docker — nothing installed locally
docker run -v "$PWD:/sources" --env JVM_VERSION=17 \
  ghcr.io/scip-code/scip-java:latest scip-java index

# or, installed locally
scip-java index
```

Read [Gradle configuration cache](#gradle-configuration-cache) below before
running this on a Gradle project — it fails with a misleading error otherwise.

Arbor can drive this one for you: `arbor scip --background`.

### Rust — [`rust-analyzer`]

Already installed if you use `rustup component add rust-analyzer` or the VS Code
extension. No build needed; it loads the workspace like an editor would.

```bash
rust-analyzer scip .
arbor scip index.scip --root .
```

Took 11s on Arbor's own 69-file workspace. Note that rust-analyzer emits **no
`is_implementation` relationships**, so virtual dispatch expansion contributes
nothing on Rust — trait-impl reach will not show up in blast radius.

### TypeScript, JavaScript — [`scip-typescript`]

Needs a `tsconfig.json` and installed `node_modules`.

```bash
npm install -g @sourcegraph/scip-typescript
scip-typescript index                    # or --yarn-workspaces / --pnpm-workspaces
arbor scip index.scip --root .
```

### Python — [`scip-python`]

Needs Python 3.10+, Node 16+, and your virtualenv **activated** — it reads
installed package versions from pip.

```bash
npm install -g @sourcegraph/scip-python
scip-python index . --project-name my-project    # --project-name is required
arbor scip index.scip --root .
```

### C, C++ — [`scip-clang`]

Needs a JSON compilation database, and the project built first so generated
headers exist. Binary releases for x86-64 Linux and arm64 macOS.

```bash
cmake -B build -DCMAKE_EXPORT_COMPILE_COMMANDS=ON   # or: bear -- make all
scip-clang --compdb-path=build/compile_commands.json
arbor scip index.scip --root .
```

Run it from the project root, not a subdirectory. Budget roughly 2 MB of
temporary space per translation unit.

### C# — [`scip-dotnet`]

```bash
# Docker
docker run -v "$PWD:/app" sourcegraph/scip-dotnet:latest scip-dotnet index

# or .NET 8.0 installed locally
dotnet tool install --global scip-dotnet
scip-dotnet index
```

### Go — [`scip-go`]

```bash
go install github.com/scip-code/scip-go/cmd/scip-go@latest
scip-go                                  # add --module-name / --module-version if it asks
arbor scip index.scip --root .
```

### Ruby — [`scip-ruby`]

Builds on Sorbet, so typed files index best; `# typed: false` files are
best-effort. Add the gem to your development group, then:

```bash
bundle exec scip-ruby          # or `bundle exec scip-ruby .` without sorbet/config
arbor scip index.scip --root .
```

### PHP — [`scip-php`]

Needs `composer.json`, `composer.lock`, and installed vendor dependencies.

```bash
composer require --dev davidrjenni/scip-php
vendor/bin/scip-php
arbor scip index.scip --root .
```

### Dart — [`scip-dart`]

```bash
dart pub global activate scip_dart
dart pub global run scip_dart ./
arbor scip index.scip --root .
```

## After ingesting, whatever the language

`arbor scip` writes `.arbor/scip.json`, which makes the graph
**SCIP-provenanced**. From then on no operation lets Tree-sitter overwrite it —
`arbor index` refuses, reads serve the cache, and `arbor status` reports the
source. That guard is language-agnostic, but the **automatic rebuild is not**:
it invokes `scip-java`. On any other language, a stale graph prints a warning
naming the indexer it cannot run, and refreshing is:

```bash
<your indexer>            # regenerate index.scip
arbor scip index.scip --root .
```

Set `ARBOR_NO_AUTO_REBUILD=1` to suppress the rebuild attempt entirely — worth
doing in CI and in editor integrations, where a read command turning into a
compile is not acceptable.

[SCIP]: https://github.com/scip-code/scip
[`scip-java`]: https://github.com/scip-code/scip-java
[`rust-analyzer`]: https://github.com/rust-lang/rust-analyzer
[`scip-typescript`]: https://github.com/sourcegraph/scip-typescript
[`scip-python`]: https://github.com/sourcegraph/scip-python
[`scip-clang`]: https://github.com/sourcegraph/scip-clang
[`scip-dotnet`]: https://github.com/sourcegraph/scip-dotnet
[`scip-go`]: https://github.com/scip-code/scip-go
[`scip-ruby`]: https://github.com/sourcegraph/scip-ruby
[`scip-php`]: https://github.com/davidrjenni/scip-php
[`scip-dart`]: https://github.com/Workiva/scip-dart

## Querying it

Nothing changes at the query layer. Same commands, same output shape — the edges
are just exact:

```bash
arbor callees "pay" .
arbor callers "charge" .
arbor map . --exclude-test
```

## scip-java specifics

### Gradle configuration cache

If `gradle.properties` sets `org.gradle.configuration-cache=true`, the index
build fails: scip-java's `WriteDependencies` task calls `Task.project` at
execution time, which the configuration cache forbids. The reported errors are
misleading — `Cannot get property 'dependenciesOut'` is a symptom, not the
cause. Disable the cache for this invocation only:

```bash
scip-java index -- clean scipPrintDependencies scipCompileAll \
  --no-configuration-cache
```

Args after `--` replace the task list rather than append to it, so all tasks
must be named. Those names are not in `./gradlew tasks` — scip-java injects
them through an init script at runtime. Read them off the `$ ./gradlew ...`
line scip-java prints when you run it bare.

## Multiple indexes

A multi-module build emits one index per module. **Pass them all in one call**, or
cross-module edges will not resolve: a symbol defined in module B is only linkable
while B's definitions are in scope, and ingesting modules one at a time drops
every edge that crosses a module boundary.

```bash
scripts/arbor-scip-ingest.sh
```

That script finds every `index.scip`, skips the zero-byte ones a failed build
leaves behind, and passes the rest in a single `arbor scip` call. The obvious
shell one-liner is worse than it looks:

```bash
arbor scip $(find . -name 'index.scip') --root .   # word-splits on spaces;
                                                   # silently indexes nothing
                                                   # when there are no matches
```

The same rule covers two indexes from *different* indexers — a Go service and
its TypeScript client, say. Node identity is `(file, qualified name, kind)`, so
indexes covering different files simply coexist.

## What you get over Tree-sitter

| | Tree-sitter | SCIP |
|---|---|---|
| `foo()` | resolved by name, confidence-weighted | exact |
| `obj.method()` | **no edge** | exact |
| Overloads | collapsed into one node | distinct nodes |
| Interface → impl | not represented | `Implements` edges |
| Virtual dispatch | not represented | synthesised call edges |
| Needs a build | no | yes |

## Flags

| Flag | Effect |
|---|---|
| `--root <path>` | Project root the index's relative paths hang off. SCIP paths are relative; Arbor stores absolute, and joining here is what lets `arbor diff` and `arbor file-graph` work on the result. |
| `--merge` | Also Tree-sitter-index the files the SCIP index does not cover, for polyglot repositories. |
| `--no-dispatch` | Skip virtual dispatch expansion. Use when you want only what the compiler literally recorded. |
| `--json` | Machine-readable stats. |

## Virtual dispatch expansion

The part that types alone do not buy you. A call to `PaymentGateway.charge()`
lands at runtime in `StripeGateway.charge()`; a call graph that stops at the
interface reports a blast radius of one for a change that reaches every
implementation.

SCIP records `is_implementation` relationships, so the override hierarchy is
available without any analysis of Arbor's own. `arbor scip` walks it and
synthesises the extra call edges:

- Followed **transitively** to depth 4 — `interface → AbstractFoo → FooImpl` is
  three levels and entirely ordinary in Spring code.
- Confidence `1/n` for `n` candidates. One implementation is a certainty
  (confidence `1.0`); twelve is a shrug.
- Dropped entirely past **8** candidates. A `Runnable` in a large codebase has
  hundreds of implementations, and "calls one of these" carries no information.
- The original edge to the declaring symbol is **kept**. It is still true, and
  dropping it would lose the fact that the code was written against the
  abstraction.

`arbor callees` and `arbor callers` will therefore show both the interface
method and its implementations. Filter on confidence if you want only the
certainties.

## Staleness

`arbor scip` writes `.arbor/scip.json` recording which indexes the graph came
from. While that marker exists, editing a source file does **not** trigger an
automatic Tree-sitter re-index — the cached graph is served with a warning
instead:

```
⚠ Sources changed since this graph was built from a SCIP index.
  Re-run `arbor scip index.scip` to refresh it; serving the cached graph for now.
```

This is deliberate. A silent rebuild would replace compiler-resolved edges with
guessed ones, which is a downgrade nobody asked for. Re-run your build and
`arbor scip` to refresh; `arbor index` clears the marker and goes back to
Tree-sitter.

## Reading the stats

Measured on a real 406-document Spring service (31 MB index):

```
✓ Ingested 406 documents from scip-java 0.0.0-SNAPSHOT (java)
  10576 definitions across 402 files
  41526 references resolved, 170148 external (JDK, jars, packages), 2055 unattributed
  26767 references ignored (locals, params, type params), 594 self/recursive, 0 unreadable
  837 implements/override edges, 3091 added by dispatch expansion
✓ Graph: 10576 nodes, 45420 edges (43397 confident)
```

The reference buckets are exhaustive by construction:
`resolved + external + ignored + unattributed + self + unreadable` equals every
non-definition occurrence in the index — 241,090 above, residual zero. That
invariant is asserted in `arbor-scip`'s tests, because an unaccounted
remainder is indistinguishable from a decoding bug.

- **definitions** will be far *lower* than the raw occurrence count in the
  index, and that is correct. The index above holds 28,320 definitions, but
  17,617 are locals and 124 are type parameters — neither belongs in a call
  graph. What is left is 6,615 methods, 3,363 fields, and 601 types.
- **external** — references to symbols that could have been nodes but have no
  definition here: the JDK, third-party jars, and package names. Expected to
  dominate; they are dropped rather than added as leaf vertices no query wants.
- **ignored** — references to locals, parameters, and type parameters. Counted
  separately from `external` on purpose: folding them together overstates how
  much the index is failing to cover.
- **self/recursive** — the reference target is the enclosing definition itself.
  Arbor drops self-edges deliberately: they add no reachability and skew
  centrality toward whatever recurses.
- **unreadable** — a position Arbor could not decode in either encoding. Should
  be zero; a non-zero count means the producer writes a range form Arbor does
  not know, and those references are silently missing from the graph.
- **unattributed** — references the indexer placed outside every definition
  body (a package declaration, a file-level annotation). There is no caller to
  hang an edge on. A large count here is worth investigating.
- **documents with no enclosing ranges** — if warned about, the indexer supplied
  no body extents for those files, so the *caller* of each edge was attributed
  by position rather than by the indexer. Still usually right; not exact. Above
  it is 4 of 406.

## Limitations

- **Usually requires a successful build.** Most of these indexers are compiler
  plugins: if the project does not compile, there is no index. An empty build
  produces an empty index, which `arbor scip` rejects rather than silently
  accept. `rust-analyzer` is the exception — it loads the workspace the way an
  editor does, so it produces an index from code that does not build.
- **No incremental update.** There is no `--changed-only` equivalent: re-run the
  indexer and `arbor scip` again. `arbor watch` does not refresh a SCIP graph.
- **Automatic refresh is scip-java only.** The staleness *check* covers every
  supported language, but the rebuild it triggers invokes `scip-java`. On other
  languages Arbor warns and serves the cached graph; refreshing is manual.
- **Dispatch expansion depends on the indexer.** It is driven by SCIP
  `is_implementation` relationships, and not every indexer emits them —
  `rust-analyzer` emits none, so trait-impl reach is absent on Rust. This
  degrades to no expansion rather than to wrong edges.
- **One graph, one provenance.** `.arbor/scip.json` is repo-wide. A polyglot
  repository cannot have SCIP-indexed Kotlin and Tree-sitter-parsed TypeScript
  in the same graph: either index both, or accept that the unindexed half is
  represented by whatever the SCIP indexes happen to cover.
- **Reflection and DI wiring are invisible.** Spring `@Autowired` injection
  points and anything reached by reflection are not calls in the bytecode sense
  and are not in the index. Dispatch expansion covers the common
  interface-injection case indirectly, but a bean resolved purely by name is
  not represented.
- **Visibility is unknown.** SCIP carries no portable notion of access
  modifiers, so every SCIP node has the default visibility rather than a guess.
- **Two range encodings exist.** SCIP's original `repeated int32 range` is
  deprecated in favour of a typed `single_line_range` / `multi_line_range`
  oneof, and current `scip-java` emits only the typed form. Arbor reads both,
  typed first. If a future producer invents a third, definitions will be
  skipped with an `unreadable range` warning rather than silently mislocated.

## Other JVM tooling considered

For context on why SCIP rather than an alternative:

- **[jQAssistant]** — scans bytecode into Neo4j, query with Cypher. Closest
  match to Arbor's shape for the JVM, but a different query model entirely.
- **[Joern]** — Code Property Graph (AST + CFG + PDG), with real dataflow.
  Strictly more powerful and strictly heavier.
- **CodeQL** — best query ergonomics and taint tracking; licensing restricts
  closed-source commercial use.
- **[OpenRewrite]** — type-attributed *and* format-preserving LSTs, aimed at
  automated refactoring rather than read-only navigation.
- **Soot / WALA / Doop** — whole-program points-to analysis. The most precise
  dispatch resolution available, at research-grade cost.

SCIP won on being an open, stable, language-agnostic *format* — Arbor ingests it
without depending on any one vendor's engine, and the same code path reads all
ten indexers listed above. That is not a hope: `rust-analyzer`'s output was run
through it unchanged, on Arbor's own source.

[jQAssistant]: https://jqassistant.org/
[Joern]: https://github.com/joernio/joern
[OpenRewrite]: https://docs.openrewrite.org/concepts-and-explanations/lossless-semantic-trees
