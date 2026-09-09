//! Arbor SCIP — compiler-accurate graphs for JVM languages.
//!
//! Arbor's own parsers read source text with Tree-sitter. That is fast and
//! needs no build, but it cannot resolve `repository.findOne()` to a
//! definition, because doing so requires knowing the type of `repository` —
//! and type inference is a compiler's job, not a grammar's.
//!
//! Rather than reimplement javac, this crate consumes [SCIP], the open code
//! indexing format, as produced by [`scip-java`] for Java, Kotlin, and Scala.
//! Those indexers run as compiler plugins, so their symbol resolution is
//! exactly the compiler's. Arbor keeps the layers it is actually good at —
//! ranking, entry-point detection, context slicing, MCP — and stops guessing
//! at the parts a compiler already knows.
//!
//! [SCIP]: https://github.com/scip-code/scip
//! [`scip-java`]: https://github.com/scip-code/scip-java
//!
//! # What this buys over Tree-sitter
//!
//! - **Resolved method calls.** `obj.method()` becomes a real edge. Tree-sitter
//!   records it as the reference `obj.method`, which matches no symbol — the
//!   receiver is a variable, not a type — so it never resolves. That is the
//!   single largest gap in Arbor's JVM coverage.
//! - **Virtual dispatch.** SCIP records the override hierarchy, so a call to
//!   `PaymentGateway.charge()` also reaches `StripeGateway.charge()`. A call
//!   graph that stops at the interface understates blast radius on any
//!   interface-driven codebase — which is most JVM code.
//! - **Honest confidence.** Every edge here is compiler-proven and carries
//!   confidence `1.0`. Only dispatch expansion goes lower, weighted by how
//!   many implementations are in play.
//!
//! # Usage
//!
//! ```no_run
//! use arbor_scip::{ingest_file, IngestOptions};
//! use std::path::Path;
//!
//! let options = IngestOptions::new("/path/to/repo");
//! let ingest = ingest_file(Path::new("index.scip"), &options)?;
//!
//! let mut builder = arbor_graph::GraphBuilder::new();
//! builder.add_nodes(ingest.nodes);
//! builder.add_pinned_edges(ingest.edges);
//! let graph = builder.build();
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # Producing an index
//!
//! ```text
//! # Maven or Gradle, from the repository root
//! docker run -v $PWD:/sources --env JVM_VERSION=17 \
//!   ghcr.io/scip-code/scip-java:latest scip-java index
//! ```
//!
//! The `scip-code/scip-java` fork emits *typed* occurrence ranges (SCIP 0.9
//! fields 8-11). Arbor reads the deprecated `repeated int32 range` too, but
//! only the typed encoding is exercised against real indexes.
//!
//! Multi-module builds emit one index per module; hand them all to
//! [`ingest_files`] so cross-module edges resolve.

pub mod dispatch;
pub mod edge;
pub mod error;
pub mod ingest;
pub mod ranges;
pub mod symbols;

pub use dispatch::ImplementationMap;
pub use edge::SymbolEdge;
pub use error::{Result, ScipError};
pub use ingest::{ingest_file, ingest_files, ingest_indexes, IngestOptions, ScipIngest, ScipStats};
pub use ranges::Span;
pub use symbols::SymbolFacts;
