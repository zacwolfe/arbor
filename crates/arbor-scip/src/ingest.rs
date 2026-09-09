//! Turning a SCIP index into Arbor nodes and exact edges.
//!
//! Two passes, for the same reason `arbor-graph`'s builder uses two: an edge's
//! kind depends on what its *target* is (calling a method is not the same as
//! naming a type), and the target may be defined in a document we have not
//! read yet.
//!
//!   1. Every `Definition` occurrence becomes a [`CodeNode`], recording
//!      `symbol → (node id, kind)`.
//!   2. Every other occurrence becomes an edge from its enclosing definition,
//!      typed by the target's kind from pass 1.
//!
//! References with no definition anywhere in the index are dropped. Those are
//! the JDK and third-party jars; keeping them would add thousands of leaf
//! vertices that no query ever wants.

use crate::dispatch::ImplementationMap;
use crate::edge::SymbolEdge;
use crate::error::{Result, ScipError};
use crate::ranges::{occurrence_enclosing_span, occurrence_span, EnclosingIndex, Span};
use crate::symbols;
use arbor_core::{CodeNode, NodeKind};
use arbor_graph::{Edge, EdgeKind, PinnedEdge};
use protobuf::Message;
use scip::types::{Document, Index, SymbolInformation, SymbolRole};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

/// How the ingest should interpret the index.
#[derive(Debug, Clone)]
pub struct IngestOptions {
    /// Absolute path the index's relative paths hang off.
    ///
    /// SCIP paths are relative to the project root; Arbor's other indexers
    /// store absolute paths. Joining here is what lets a SCIP graph and a
    /// Tree-sitter graph refer to the same file — and therefore lets
    /// `arbor diff` and `arbor file-graph` work on a SCIP-built graph.
    pub project_root: PathBuf,

    /// Whether to synthesise call edges for virtual dispatch.
    ///
    /// On by default: without it, a call graph over interface-heavy JVM code
    /// reports a blast radius that stops at the interface.
    pub expand_dispatch: bool,
}

impl IngestOptions {
    pub fn new(project_root: impl Into<PathBuf>) -> Self {
        Self {
            project_root: project_root.into(),
            expand_dispatch: true,
        }
    }

    pub fn without_dispatch_expansion(mut self) -> Self {
        self.expand_dispatch = false;
        self
    }
}

/// What one or more SCIP indexes yielded.
#[derive(Debug)]
pub struct ScipIngest {
    /// Definitions, ready for [`arbor_graph::GraphBuilder::add_nodes`].
    ///
    /// `references` is deliberately left empty: these nodes' edges are exact
    /// and arrive via [`ScipIngest::edges`]. Populating `references` would
    /// invite name-based resolution to add a second, guessed edge for every
    /// one we already know precisely.
    pub nodes: Vec<CodeNode>,

    /// Exact edges, ready for [`arbor_graph::GraphBuilder::add_pinned_edges`].
    pub edges: Vec<PinnedEdge>,

    pub stats: ScipStats,
}

/// Counts worth reporting, so a user can tell an exact result from a
/// heuristic one rather than having to trust the graph blindly.
#[derive(Debug, Default, Clone)]
pub struct ScipStats {
    /// Indexer that produced the input, e.g. `scip-java 0.10.3`.
    pub tools: Vec<String>,
    pub documents: usize,
    pub definitions: usize,
    /// References resolved to a definition inside the index.
    pub references_resolved: usize,
    /// References to symbols that *could* have been nodes but have no
    /// definition in this index: the JDK, third-party jars, and package names.
    pub references_external: usize,
    /// References to symbols Arbor never makes nodes from — locals,
    /// parameters, type parameters. Counted separately because folding them
    /// into `references_external` overstates how much the index is missing.
    pub references_ignored: usize,
    /// References we could not attribute to an enclosing definition.
    pub references_unattributed: usize,
    /// References whose target is the enclosing definition itself.
    ///
    /// Recursion, and the declaration site's own name where the indexer emits
    /// it as a reference too. Arbor drops self-edges deliberately — they add
    /// no reachability and skew centrality toward whatever recurses — but they
    /// are counted so the buckets close.
    pub references_self: usize,
    /// References whose position we could not read in either encoding.
    ///
    /// Counted rather than dropped silently so the reference buckets add up:
    /// `resolved + external + ignored + unattributed + without_range` equals
    /// every non-definition occurrence in the index. An unaccounted remainder
    /// is indistinguishable from a decoding bug — which is exactly how the
    /// deprecated-`range` regression hid.
    pub references_without_range: usize,
    /// `Implements`/override edges taken straight from SCIP relationships.
    pub implements_edges: usize,
    /// Call edges synthesised by dispatch expansion.
    pub dispatch_edges: usize,
    /// Documents where the indexer gave no body extents, so enclosing-symbol
    /// attribution fell back to the nearest-preceding-definition heuristic.
    pub documents_without_body_extents: usize,
    /// Languages seen, as reported by the indexer.
    pub languages: Vec<String>,
}

/// A definition we have already turned into a node.
struct DefinedSymbol {
    node_id: String,
    kind: NodeKind,
    file: String,
    line: u32,
}

/// Reads and ingests a single SCIP index.
pub fn ingest_file(path: &Path, options: &IngestOptions) -> Result<ScipIngest> {
    ingest_files(std::slice::from_ref(&path.to_path_buf()), options)
}

/// Reads and ingests several SCIP indexes into one graph.
///
/// Multi-module JVM builds emit one index per module; merging them here is
/// what makes cross-module edges resolvable at all.
pub fn ingest_files(paths: &[PathBuf], options: &IngestOptions) -> Result<ScipIngest> {
    let indexes = paths
        .iter()
        .map(|path| read_index(path).map(|index| (path.clone(), index)))
        .collect::<Result<Vec<_>>>()?;

    Ok(ingest_indexes(&indexes, options))
}

/// Decodes one SCIP protobuf payload.
fn read_index(path: &Path) -> Result<Index> {
    let bytes = std::fs::read(path).map_err(|source| ScipError::Io {
        path: path.to_path_buf(),
        source,
    })?;

    let index = Index::parse_from_bytes(&bytes).map_err(|error| ScipError::Decode {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;

    if index.documents.is_empty() {
        return Err(ScipError::Empty {
            path: path.to_path_buf(),
        });
    }

    Ok(index)
}

/// The ingest proper, on already-decoded indexes.
///
/// Split out from [`ingest_files`] so tests can build an [`Index`] in memory
/// instead of round-tripping through a fixture file.
pub fn ingest_indexes(indexes: &[(PathBuf, Index)], options: &IngestOptions) -> ScipIngest {
    let mut stats = ScipStats::default();
    let mut nodes = Vec::new();
    let mut defined: HashMap<String, DefinedSymbol> = HashMap::new();

    for (_, index) in indexes {
        if let Some(tool) = index.metadata.tool_info.as_ref() {
            let label = match tool.version.is_empty() {
                true => tool.name.clone(),
                false => format!("{} {}", tool.name, tool.version),
            };
            if !label.is_empty() && !stats.tools.contains(&label) {
                stats.tools.push(label);
            }
        }

        for document in &index.documents {
            stats.documents += 1;
            if !document.language.is_empty() && !stats.languages.contains(&document.language) {
                stats.languages.push(document.language.clone());
            }

            let style = symbols::style_for(
                &document.language,
                document.occurrences.first().map(|o| o.symbol.as_str()),
            );

            collect_definitions(document, options, &style, &mut nodes, &mut defined);
        }
    }

    stats.definitions = nodes.len();

    // Pass 2: references become edges, now that every definition's kind is
    // known regardless of which document it lives in.
    let mut symbol_edges = Vec::new();
    for (_, index) in indexes {
        for document in &index.documents {
            let enclosing = build_enclosing_index(document);
            if !enclosing.has_body_extents() {
                stats.documents_without_body_extents += 1;
            }

            collect_references(
                document,
                &enclosing,
                &defined,
                &mut symbol_edges,
                &mut stats,
            );
        }
    }

    let implementations = build_implementation_map(indexes);

    let implements_edges = implementation_edges(&implementations, &defined);
    stats.implements_edges = implements_edges.len();
    symbol_edges.extend(implements_edges);

    if options.expand_dispatch {
        let expanded = implementations.expand(&symbol_edges);
        stats.dispatch_edges = expanded.len();
        symbol_edges.extend(expanded);
    }

    let edges = to_pinned_edges(symbol_edges, &defined);

    ScipIngest {
        nodes,
        edges,
        stats,
    }
}

/// Pass 1 for one document.
fn collect_definitions(
    document: &Document,
    options: &IngestOptions,
    style: &symbols::SymbolStyle,
    nodes: &mut Vec<CodeNode>,
    defined: &mut HashMap<String, DefinedSymbol>,
) {
    let file = absolute_file(&options.project_root, &document.relative_path);
    let info_by_symbol = index_symbol_information(document);

    for occurrence in &document.occurrences {
        if !has_role(occurrence.symbol_roles, SymbolRole::Definition) {
            continue;
        }

        let Some(facts) = symbols::parse(&occurrence.symbol, style) else {
            continue;
        };

        // The same symbol defined twice means two indexes covered the same
        // module. Keeping the first is right; adding the second would create
        // a duplicate vertex with an identical node ID, which the graph's
        // ID index cannot represent.
        if defined.contains_key(&occurrence.symbol) {
            debug!(
                "Duplicate definition of {} in {}, keeping the first",
                occurrence.symbol, file
            );
            continue;
        }

        let Some(name_span) = occurrence_span(occurrence) else {
            warn!(
                "Skipping definition {} in {}: unreadable range",
                occurrence.symbol, file
            );
            continue;
        };

        let info = info_by_symbol.get(occurrence.symbol.as_str()).copied();
        let kind = symbols::refine_kind(facts.kind, info);
        let body_span = occurrence_enclosing_span(occurrence).unwrap_or(name_span);

        let node = build_node(&facts, kind, &file, name_span, body_span, info);

        defined.insert(
            occurrence.symbol.clone(),
            DefinedSymbol {
                node_id: node.id.clone(),
                kind,
                file: file.clone(),
                line: name_span.start_line,
            },
        );

        nodes.push(node);
    }
}

fn build_node(
    facts: &symbols::SymbolFacts,
    kind: NodeKind,
    file: &str,
    name_span: Span,
    body_span: Span,
    info: Option<&SymbolInformation>,
) -> CodeNode {
    let display_name = info
        .map(|i| i.display_name.as_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(&facts.simple_name);

    // Visibility is left at its default: a SCIP index carries no portable
    // notion of access modifiers, and guessing would be worse than admitting
    // we do not know.
    let mut node = CodeNode::new(display_name, &facts.qualified_name, kind, file).with_lines(
        name_span.start_line,
        body_span.end_line.max(name_span.end_line),
    );

    if let Some(info) = info {
        if let Some(signature) = info
            .signature_documentation
            .as_ref()
            .map(|sig| sig.text.as_str())
            .filter(|text| !text.is_empty())
        {
            node = node.with_signature(signature);
        }

        let documentation = info
            .documentation
            .iter()
            .filter(|doc| !doc.is_empty())
            .cloned()
            .collect::<Vec<_>>();

        if !documentation.is_empty() {
            node.docstring = Some(documentation.join("\n"));
        }
    }

    node
}

/// Pass 2 for one document.
fn collect_references(
    document: &Document,
    enclosing: &EnclosingIndex,
    defined: &HashMap<String, DefinedSymbol>,
    edges: &mut Vec<SymbolEdge>,
    stats: &mut ScipStats,
) {
    let file = document.relative_path.clone();

    for occurrence in &document.occurrences {
        if has_role(occurrence.symbol_roles, SymbolRole::Definition) {
            continue;
        }
        // Locals, parameters, and type parameters are never graph vertices, so
        // a reference to one is not a gap in the index — it is simply not our
        // business. Filtering here keeps `references_external` meaning
        // "something we could have linked but the index does not define".
        if !symbols::is_graph_symbol(&occurrence.symbol) {
            stats.references_ignored += 1;
            continue;
        }

        let Some(target) = defined.get(&occurrence.symbol) else {
            stats.references_external += 1;
            continue;
        };

        let Some(span) = occurrence_span(occurrence) else {
            stats.references_without_range += 1;
            continue;
        };

        let Some(from_symbol) = enclosing.resolve(span.start_line) else {
            // A reference outside any definition — a package declaration, or a
            // file-level annotation. There is no caller to attribute it to.
            stats.references_unattributed += 1;
            continue;
        };

        if from_symbol == occurrence.symbol {
            stats.references_self += 1;
            continue;
        }

        let kind = reference_edge_kind(occurrence.symbol_roles, target.kind);
        stats.references_resolved += 1;

        edges.push(SymbolEdge::exact(
            from_symbol,
            &occurrence.symbol,
            kind,
            &file,
            span.start_line,
        ));
    }
}

/// What kind of edge a reference implies, from the target's kind.
///
/// The distinction matters downstream: `arbor callers` and blast-radius
/// traversal weight a call differently from a bare type mention.
fn reference_edge_kind(roles: i32, target_kind: NodeKind) -> EdgeKind {
    if has_role(roles, SymbolRole::Import) {
        return EdgeKind::Imports;
    }

    match target_kind {
        kind if symbols::is_callable(kind) => EdgeKind::Calls,
        NodeKind::Class
        | NodeKind::Interface
        | NodeKind::Enum
        | NodeKind::Struct
        | NodeKind::TypeAlias => EdgeKind::UsesType,
        NodeKind::Module => EdgeKind::Imports,
        _ => EdgeKind::References,
    }
}

/// Builds the enclosing-definition index for one document.
fn build_enclosing_index(document: &Document) -> EnclosingIndex {
    let definitions = document
        .occurrences
        .iter()
        .filter(|occurrence| has_role(occurrence.symbol_roles, SymbolRole::Definition))
        .filter(|occurrence| symbols::is_graph_symbol(&occurrence.symbol))
        .filter_map(|occurrence| {
            let name_span = occurrence_span(occurrence)?;
            let body_span = occurrence_enclosing_span(occurrence);
            Some((name_span, body_span, occurrence.symbol.clone()))
        });

    EnclosingIndex::build(definitions)
}

/// Collects `is_implementation` relationships from every index.
///
/// `external_symbols` is included on purpose: an interface declared in a
/// dependency still has implementations in this repository, and dropping
/// those relationships would silently disable dispatch expansion for every
/// framework interface — which is most of them in Spring code.
fn build_implementation_map(indexes: &[(PathBuf, Index)]) -> ImplementationMap {
    let pairs = indexes
        .iter()
        .flat_map(|(_, index)| {
            let from_documents = index.documents.iter().flat_map(|d| d.symbols.iter());
            from_documents.chain(index.external_symbols.iter())
        })
        .flat_map(|info| {
            info.relationships
                .iter()
                .filter(|relationship| relationship.is_implementation)
                .map(move |relationship| (info.symbol.clone(), relationship.symbol.clone()))
        });

    ImplementationMap::from_pairs(pairs)
}

/// Turns the override hierarchy into `Implements` edges.
///
/// Only pairs where both ends are defined here become edges; an override of a
/// JDK method has nothing local to point at.
fn implementation_edges(
    implementations: &ImplementationMap,
    defined: &HashMap<String, DefinedSymbol>,
) -> Vec<SymbolEdge> {
    defined
        .iter()
        .flat_map(|(symbol, definition)| {
            implementations
                .direct_implementations(symbol)
                .iter()
                .filter(|implementor| defined.contains_key(implementor.as_str()))
                .map(move |implementor| {
                    // Direction is implementor → supertype: the concrete class
                    // depends on the abstraction, not the reverse.
                    SymbolEdge::exact(
                        implementor,
                        symbol,
                        EdgeKind::Implements,
                        &definition.file,
                        definition.line,
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Converts symbol-space edges into node-ID-space edges.
fn to_pinned_edges(
    edges: Vec<SymbolEdge>,
    defined: &HashMap<String, DefinedSymbol>,
) -> Vec<PinnedEdge> {
    edges
        .into_iter()
        .filter_map(|symbol_edge| {
            let from = defined.get(&symbol_edge.from)?;
            let to = defined.get(&symbol_edge.to)?;

            Some(PinnedEdge {
                from_id: from.node_id.clone(),
                to_id: to.node_id.clone(),
                edge: Edge::with_location(symbol_edge.kind, symbol_edge.file, symbol_edge.line)
                    .with_confidence(symbol_edge.confidence),
            })
        })
        .collect()
}

/// Indexes a document's `SymbolInformation` by symbol string.
fn index_symbol_information(document: &Document) -> HashMap<&str, &SymbolInformation> {
    document
        .symbols
        .iter()
        .map(|info| (info.symbol.as_str(), info))
        .collect()
}

/// Whether a role bit is set in an occurrence's bitmask.
fn has_role(roles: i32, role: SymbolRole) -> bool {
    roles & (role as i32) != 0
}

/// Joins a document-relative path onto the project root.
fn absolute_file(project_root: &Path, relative_path: &str) -> String {
    project_root.join(relative_path).display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use scip::types::{
        occurrence, MultiLineRange, Occurrence, Relationship, SingleLineRange, ToolInfo,
    };

    const GATEWAY: &str = "semanticdb maven . . com/example/Gateway#charge().";
    const STRIPE: &str = "semanticdb maven . . com/example/StripeGateway#charge().";
    const CHECKOUT: &str = "semanticdb maven . . com/example/Checkout#pay().";

    fn single_line(line: i32) -> SingleLineRange {
        let mut range = SingleLineRange::new();
        range.line = line;
        range.start_character = 2;
        range.end_character = 20;
        range
    }

    fn multi_line(start: i32, end: i32) -> MultiLineRange {
        let mut range = MultiLineRange::new();
        range.start_line = start;
        range.start_character = 2;
        range.end_line = end;
        range.end_character = 3;
        range
    }

    /// Built with the *typed* encoding, because that is the only one current
    /// `scip-java` emits. A fixture using the deprecated `range` array would
    /// pass while the real thing produced an empty graph.
    fn definition(symbol: &str, start_line: i32, end_line: i32) -> Occurrence {
        let mut occurrence = Occurrence::new();
        occurrence.symbol = symbol.to_string();
        occurrence.symbol_roles = SymbolRole::Definition as i32;
        occurrence.typed_range = Some(occurrence::Typed_range::SingleLineRange(single_line(
            start_line,
        )));
        occurrence.typed_enclosing_range =
            Some(occurrence::Typed_enclosing_range::MultiLineEnclosingRange(
                multi_line(start_line, end_line),
            ));
        occurrence
    }

    fn reference(symbol: &str, line: i32) -> Occurrence {
        let mut occurrence = Occurrence::new();
        occurrence.symbol = symbol.to_string();
        occurrence.typed_range = Some(occurrence::Typed_range::SingleLineRange(single_line(line)));
        occurrence
    }

    /// The deprecated `repeated int32` encoding, for indexes predating the
    /// typed oneof. Ingest must still read them.
    fn legacy_definition(symbol: &str, start_line: i32, end_line: i32) -> Occurrence {
        let mut occurrence = Occurrence::new();
        occurrence.symbol = symbol.to_string();
        occurrence.symbol_roles = SymbolRole::Definition as i32;
        occurrence.range = vec![start_line, 2, start_line, 20];
        occurrence.enclosing_range = vec![start_line, 2, end_line, 3];
        occurrence
    }

    fn legacy_reference(symbol: &str, line: i32) -> Occurrence {
        let mut occurrence = Occurrence::new();
        occurrence.symbol = symbol.to_string();
        occurrence.range = vec![line, 8, 30];
        occurrence
    }

    fn symbol_info(symbol: &str, implements: &[&str]) -> SymbolInformation {
        let mut info = SymbolInformation::new();
        info.symbol = symbol.to_string();
        info.relationships = implements
            .iter()
            .map(|target| {
                let mut relationship = Relationship::new();
                relationship.symbol = target.to_string();
                relationship.is_implementation = true;
                relationship
            })
            .collect();
        info
    }

    fn document(
        path: &str,
        occurrences: Vec<Occurrence>,
        symbols: Vec<SymbolInformation>,
    ) -> Document {
        let mut document = Document::new();
        document.relative_path = path.to_string();
        document.language = "Java".to_string();
        document.occurrences = occurrences;
        document.symbols = symbols;
        document
    }

    fn index(documents: Vec<Document>) -> Index {
        let mut index = Index::new();
        let mut tool = ToolInfo::new();
        tool.name = "scip-java".to_string();
        tool.version = "0.10.3".to_string();
        index.metadata.mut_or_insert_default().tool_info = Some(tool).into();
        index.documents = documents;
        index
    }

    fn run(documents: Vec<Document>) -> ScipIngest {
        let options = IngestOptions::new("/repo");
        ingest_indexes(&[(PathBuf::from("index.scip"), index(documents))], &options)
    }

    #[test]
    fn definitions_become_nodes_with_absolute_paths() {
        let result = run(vec![document(
            "src/main/java/com/example/Gateway.java",
            vec![definition(GATEWAY, 9, 12)],
            vec![],
        )]);

        assert_eq!(result.nodes.len(), 1);
        let node = &result.nodes[0];
        assert_eq!(node.qualified_name, "com.example.Gateway.charge");
        assert_eq!(node.kind, NodeKind::Method);
        assert_eq!(node.line_start, 10);
        assert_eq!(node.line_end, 13);
        assert!(node
            .file
            .ends_with("src/main/java/com/example/Gateway.java"));
        assert!(node.file.starts_with("/repo"));
        // Exact edges arrive separately; name resolution must not double up.
        assert!(node.references.is_empty());
    }

    #[test]
    fn reference_becomes_a_call_edge_from_its_enclosing_definition() {
        let result = run(vec![
            document("Gateway.java", vec![definition(GATEWAY, 9, 12)], vec![]),
            document(
                "Checkout.java",
                vec![definition(CHECKOUT, 4, 8), reference(GATEWAY, 6)],
                vec![],
            ),
        ]);

        assert_eq!(result.stats.references_resolved, 1);
        let calls: Vec<_> = result
            .edges
            .iter()
            .filter(|e| e.edge.kind == EdgeKind::Calls)
            .collect();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].edge.confidence, 1.0);
        assert_eq!(calls[0].edge.line, Some(7));
    }

    #[test]
    fn references_to_undefined_symbols_are_dropped_and_counted() {
        let result = run(vec![document(
            "Checkout.java",
            vec![
                definition(CHECKOUT, 4, 8),
                reference("semanticdb maven . . java/util/List#of().", 6),
            ],
            vec![],
        )]);

        assert!(result.edges.is_empty());
        assert_eq!(result.stats.references_external, 1);
        assert_eq!(result.stats.references_resolved, 0);
        assert_eq!(result.stats.references_ignored, 0);
    }

    #[test]
    fn references_to_locals_are_ignored_not_reported_as_missing() {
        let result = run(vec![document(
            "Checkout.java",
            vec![
                definition(CHECKOUT, 4, 8),
                reference("local 12", 6),
                reference("semanticdb maven . . com/example/Svc#find().[T]", 6),
            ],
            vec![],
        )]);

        assert!(result.edges.is_empty());
        assert_eq!(
            result.stats.references_external, 0,
            "a local is not a missing definition"
        );
        assert_eq!(result.stats.references_ignored, 2);
    }

    #[test]
    fn dispatch_expansion_reaches_the_implementation() {
        let result = run(vec![
            document("Gateway.java", vec![definition(GATEWAY, 9, 12)], vec![]),
            document(
                "StripeGateway.java",
                vec![definition(STRIPE, 14, 20)],
                vec![symbol_info(STRIPE, &[GATEWAY])],
            ),
            document(
                "Checkout.java",
                vec![definition(CHECKOUT, 4, 8), reference(GATEWAY, 6)],
                vec![],
            ),
        ]);

        // The interface method keeps its edge, and the implementation gains one.
        let call_targets: Vec<&str> = result
            .edges
            .iter()
            .filter(|e| e.edge.kind == EdgeKind::Calls)
            .map(|e| e.to_id.as_str())
            .collect();
        assert_eq!(call_targets.len(), 2);

        assert_eq!(result.stats.dispatch_edges, 1);
        assert_eq!(result.stats.implements_edges, 1);
        assert!(result
            .edges
            .iter()
            .any(|e| e.edge.kind == EdgeKind::Implements));
    }

    #[test]
    fn dispatch_expansion_can_be_switched_off() {
        let options = IngestOptions::new("/repo").without_dispatch_expansion();
        let documents = vec![
            document("Gateway.java", vec![definition(GATEWAY, 9, 12)], vec![]),
            document(
                "StripeGateway.java",
                vec![definition(STRIPE, 14, 20)],
                vec![symbol_info(STRIPE, &[GATEWAY])],
            ),
            document(
                "Checkout.java",
                vec![definition(CHECKOUT, 4, 8), reference(GATEWAY, 6)],
                vec![],
            ),
        ];

        let result = ingest_indexes(&[(PathBuf::from("index.scip"), index(documents))], &options);

        assert_eq!(result.stats.dispatch_edges, 0);
        // Implements edges are facts from the index, not inference, so they stay.
        assert_eq!(result.stats.implements_edges, 1);
    }

    #[test]
    fn duplicate_definitions_across_indexes_collapse_to_one_node() {
        let options = IngestOptions::new("/repo");
        let one = index(vec![document(
            "Gateway.java",
            vec![definition(GATEWAY, 9, 12)],
            vec![],
        )]);
        let two = index(vec![document(
            "Gateway.java",
            vec![definition(GATEWAY, 9, 12)],
            vec![],
        )]);

        let result = ingest_indexes(
            &[
                (PathBuf::from("a.scip"), one),
                (PathBuf::from("b.scip"), two),
            ],
            &options,
        );

        assert_eq!(result.nodes.len(), 1);
        assert_eq!(result.stats.definitions, 1);
    }

    #[test]
    fn unattributed_references_are_counted_not_guessed() {
        // The target is defined, so this is not an external reference — but it
        // sits outside every body extent in its own file, so there is no
        // caller to hang the edge on.
        let result = run(vec![
            document("Gateway.java", vec![definition(GATEWAY, 9, 12)], vec![]),
            document(
                "Checkout.java",
                vec![reference(GATEWAY, 1), definition(CHECKOUT, 9, 12)],
                vec![],
            ),
        ]);

        assert!(result.edges.is_empty());
        assert_eq!(result.stats.references_external, 0);
        assert_eq!(result.stats.references_unattributed, 1);
    }

    #[test]
    fn type_reference_is_not_a_call() {
        const GATEWAY_TYPE: &str = "semanticdb maven . . com/example/Gateway#";

        let result = run(vec![
            document(
                "Gateway.java",
                vec![definition(GATEWAY_TYPE, 3, 30)],
                vec![],
            ),
            document(
                "Checkout.java",
                vec![definition(CHECKOUT, 4, 8), reference(GATEWAY_TYPE, 6)],
                vec![],
            ),
        ]);

        let kinds: Vec<EdgeKind> = result.edges.iter().map(|e| e.edge.kind).collect();
        assert_eq!(kinds, vec![EdgeKind::UsesType]);
    }

    #[test]
    fn tool_and_language_metadata_is_reported() {
        let result = run(vec![document(
            "Gateway.java",
            vec![definition(GATEWAY, 9, 12)],
            vec![],
        )]);

        assert_eq!(result.stats.tools, vec!["scip-java 0.10.3".to_string()]);
        assert_eq!(result.stats.languages, vec!["Java".to_string()]);
        assert_eq!(result.stats.documents, 1);
        assert_eq!(result.stats.documents_without_body_extents, 0);
    }

    #[test]
    fn the_deprecated_range_encoding_still_ingests() {
        let result = run(vec![
            document(
                "Gateway.java",
                vec![legacy_definition(GATEWAY, 9, 12)],
                vec![],
            ),
            document(
                "Checkout.java",
                vec![
                    legacy_definition(CHECKOUT, 4, 8),
                    legacy_reference(GATEWAY, 6),
                ],
                vec![],
            ),
        ]);

        assert_eq!(result.nodes.len(), 2);
        assert_eq!(result.stats.references_resolved, 1);
        assert_eq!(result.stats.documents_without_body_extents, 0);
    }

    /// Every non-definition occurrence must land in exactly one bucket.
    ///
    /// An unaccounted remainder looks identical to a decoding bug from the
    /// outside — it is how the deprecated-`range` regression stayed hidden
    /// while 32 tests passed. This invariant makes that impossible to miss.
    #[test]
    fn every_non_definition_occurrence_lands_in_exactly_one_bucket() {
        let mut no_range = Occurrence::new();
        no_range.symbol = GATEWAY.to_string();
        // Neither encoding set — the case that produced the unexplained residual.

        let documents = vec![
            document("Gateway.java", vec![definition(GATEWAY, 9, 12)], vec![]),
            document(
                "Checkout.java",
                vec![
                    definition(CHECKOUT, 4, 8),
                    reference(GATEWAY, 6), // resolved
                    reference("semanticdb maven . . java/util/List#of().", 6), // external
                    reference("local 12", 6), // ignored
                    reference(GATEWAY, 1), // unattributed
                    no_range,              // without_range
                    reference(CHECKOUT, 6), // self/recursive
                ],
                vec![],
            ),
        ];

        let non_definitions: usize = documents
            .iter()
            .flat_map(|d| d.occurrences.iter())
            .filter(|o| o.symbol_roles & (SymbolRole::Definition as i32) == 0)
            .count();

        let stats = run(documents).stats;
        let bucketed = stats.references_resolved
            + stats.references_external
            + stats.references_ignored
            + stats.references_unattributed
            + stats.references_without_range
            + stats.references_self;

        assert_eq!(
            bucketed, non_definitions,
            "reference buckets must sum to every non-definition occurrence"
        );
        assert_eq!(stats.references_resolved, 1);
        assert_eq!(stats.references_external, 1);
        assert_eq!(stats.references_ignored, 1);
        assert_eq!(stats.references_unattributed, 1);
        assert_eq!(stats.references_without_range, 1);
        assert_eq!(stats.references_self, 1);
    }
}
