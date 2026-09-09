# SCIP Ingestion — Compiler-Accurate Graphs for JVM Languages

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

Rather than reimplement javac, Arbor consumes [SCIP], the open code indexing
format. [`scip-java`] produces it for Java, Kotlin, and Scala by running as a
compiler plugin, so its symbol resolution *is* the compiler's. Arbor keeps the
layers it is genuinely good at — ranking, entry-point detection, context
slicing, MCP — and stops guessing at the part a compiler already knows.

[SCIP]: https://github.com/scip-code/scip
[`scip-java`]: https://github.com/scip-code/scip-java

## Quick start

```bash
# 1. Produce an index (Docker is the least invasive way; no local JDK setup)
docker run -v "$PWD:/sources" --env JVM_VERSION=17 \
  ghcr.io/scip-code/scip-java:latest scip-java index

# 2. Ingest it
arbor scip index.scip --root .

# 3. Query as usual — the graph is the same shape, the edges are just exact
arbor callees "pay" .
arbor callers "charge" .
arbor map . --exclude-test
```

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

### Multiple modules

Multi-module builds emit one index per module. Pass them all in one call, or
cross-module edges will not resolve:

```bash
arbor scip $(find . -name 'index.scip') --root .
```

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

- **Requires a successful build.** `scip-java` is a compiler plugin. If the
  project does not compile, there is no index. An empty build produces an empty
  index, which `arbor scip` rejects rather than silently accept.
- **No incremental update.** There is no `--changed-only` equivalent: re-run the
  indexer and `arbor scip` again. `arbor watch` does not refresh a SCIP graph.
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

SCIP won on being an open, stable, language-agnostic *format* — Arbor ingests
it without depending on any one vendor's engine, and the same code path picks
up `scip-typescript`, `scip-python`, and `rust-analyzer`'s SCIP output for free.

[jQAssistant]: https://jqassistant.org/
[Joern]: https://github.com/joernio/joern
[OpenRewrite]: https://docs.openrewrite.org/concepts-and-explanations/lossless-semantic-trees
