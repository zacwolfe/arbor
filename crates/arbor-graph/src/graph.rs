//! Core graph data structure.
//!
//! The ArborGraph wraps petgraph and adds indexes for fast lookups.
//! It's the central data structure that everything else works with.

use crate::edge::{Edge, EdgeKind, ExportEdge, GraphEdge, PinnedEdge};
use crate::search_index::SearchIndex;
use arbor_core::CodeNode;
use petgraph::stable_graph::{NodeIndex, StableDiGraph};
use petgraph::visit::{EdgeRef, IntoEdgeReferences}; // For edge_references
use petgraph::Direction;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// Unique identifier for a node in the graph.
pub type NodeId = NodeIndex;

/// The edge kinds that express a type hierarchy.
///
/// A single `EdgeKind` cannot say "inheritance" — a class can implement an
/// interface or extend a base class — so every hierarchy query needs both.
const INHERITANCE_KINDS: &[EdgeKind] = &[EdgeKind::Implements, EdgeKind::Extends];

/// The code relationship graph.
///
/// This is the heart of Arbor. It stores all code entities as nodes
/// and their relationships as edges, with indexes for fast access.
#[derive(Debug, Serialize, Deserialize)]
pub struct ArborGraph {
    /// The underlying petgraph graph.
    pub(crate) graph: StableDiGraph<CodeNode, Edge>,

    /// Maps string IDs to graph node indexes.
    id_index: HashMap<String, NodeId>,

    /// Maps node names to node IDs (for search).
    name_index: HashMap<String, Vec<NodeId>>,

    /// Maps file paths to node IDs (for incremental updates).
    file_index: HashMap<String, Vec<NodeId>>,

    /// Percentile-rank centrality, comparable across repositories.
    centrality: HashMap<NodeId, f64>,

    /// Raw PageRank mass, kept so a recompute can warm-start from it.
    #[serde(default)]
    centrality_raw: HashMap<NodeId, f64>,

    /// Search index for fast substring queries.
    #[serde(skip)]
    search_index: SearchIndex,
}

impl Default for ArborGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl ArborGraph {
    /// Creates a new empty graph.
    pub fn new() -> Self {
        Self {
            graph: StableDiGraph::new(),
            id_index: HashMap::new(),
            name_index: HashMap::new(),
            file_index: HashMap::new(),
            centrality: HashMap::new(),
            centrality_raw: HashMap::new(),
            search_index: SearchIndex::new(),
        }
    }

    /// Rebuilds the search index from existing graph nodes.
    /// Call after deserialization since search_index is not serialized.
    pub fn rebuild_search_index(&mut self) {
        self.search_index = SearchIndex::new();
        for index in self.graph.node_indices() {
            if let Some(node) = self.graph.node_weight(index) {
                self.search_index.insert(&node.name, index);
                index_node_documentation(&mut self.search_index, node, index);
            }
        }
    }

    /// Adds a code node to the graph.
    ///
    /// Returns the node's index for adding edges later.
    pub fn add_node(&mut self, node: CodeNode) -> NodeId {
        let id = node.id.clone();
        let name = node.name.clone();
        let file = node.file.clone();

        let index = self.graph.add_node(node);

        // Update indexes
        self.id_index.insert(id, index);
        self.name_index.entry(name.clone()).or_default().push(index);
        self.file_index.entry(file).or_default().push(index);
        self.search_index.insert(&name, index);
        if let Some(node) = self.graph.node_weight(index) {
            index_node_documentation(&mut self.search_index, node, index);
        }

        index
    }

    /// Adds an edge between two nodes.
    pub fn add_edge(&mut self, from: NodeId, to: NodeId, edge: Edge) {
        self.graph.add_edge(from, to, edge);
    }

    /// Inserts edges whose endpoints are already resolved to node IDs.
    ///
    /// Returns the number dropped because an endpoint is not in the graph. A
    /// non-zero count is normal for a compiler index — it references library
    /// symbols that have no definition in this repository — but a count near
    /// the total means the index and the node set disagree, so callers should
    /// surface it rather than discard it.
    pub fn add_pinned_edges(&mut self, edges: impl IntoIterator<Item = PinnedEdge>) -> usize {
        let mut dropped = 0usize;

        for pinned in edges {
            let (Some(from), Some(to)) = (
                self.get_index(&pinned.from_id),
                self.get_index(&pinned.to_id),
            ) else {
                dropped += 1;
                continue;
            };

            if from == to {
                // Self-recursion adds no reachability information and skews
                // centrality toward whatever happens to recurse.
                continue;
            }

            self.add_edge(from, to, pinned.edge);
        }

        dropped
    }

    /// Gets a node by its string ID.
    pub fn get_by_id(&self, id: &str) -> Option<&CodeNode> {
        let index = self.id_index.get(id)?;
        self.graph.node_weight(*index)
    }

    /// Gets a node by its graph index.
    pub fn get(&self, index: NodeId) -> Option<&CodeNode> {
        self.graph.node_weight(index)
    }

    /// Finds all nodes with a given name.
    pub fn find_by_name(&self, name: &str) -> Vec<&CodeNode> {
        self.name_index
            .get(name)
            .map(|indexes| {
                indexes
                    .iter()
                    .filter_map(|idx| self.graph.node_weight(*idx))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Resolves a symbol name to candidate nodes, best match first.
    ///
    /// Names collide constantly — `getPath` may be a utility function *and* a
    /// method on four unrelated classes. Callers that took
    /// `find_by_name(..).first()` got whichever file happened to be parsed
    /// first and then answered as though it were the only candidate; on a real
    /// codebase that reported a heavily-used function as unreachable dead code.
    ///
    /// Ranking is by graph degree, then centrality, then file path — so the
    /// pick is meaningful and does not depend on parse order. An exact node-id
    /// match short-circuits to a single result.
    pub fn resolve_symbol_ranked(&self, name: &str) -> Vec<NodeId> {
        if let Some(idx) = self.get_index(name) {
            return vec![idx];
        }

        let mut candidates: Vec<NodeId> = self
            .find_by_name(name)
            .iter()
            .filter_map(|n| self.get_index(&n.id))
            .collect();

        // The simple-name index is keyed on `name`, so a qualified name misses
        // it entirely. That made the ambiguity note's own advice — "pass a
        // qualified name to pick a specific one" — impossible to follow: every
        // name it printed came back "not found".
        if candidates.is_empty() {
            candidates = self.resolve_by_qualified_name(name);
        }

        candidates.sort_by(|&a, &b| {
            let degree = |i: NodeId| self.get_callers(i).len() + self.get_callees(i).len();
            degree(b)
                .cmp(&degree(a))
                .then_with(|| {
                    self.centrality(b)
                        .partial_cmp(&self.centrality(a))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                // With every candidate equally unconnected — common for a name
                // that is both a type annotation and a function — degree and
                // centrality decide nothing, and picking a field over the
                // function of the same name is never what was meant.
                .then_with(|| {
                    let callable =
                        |i: NodeId| self.get(i).is_some_and(|n| is_callable_kind(n.kind));
                    callable(b).cmp(&callable(a))
                })
                .then_with(|| {
                    let file = |i: NodeId| self.get(i).map(|n| n.file.clone()).unwrap_or_default();
                    file(a).cmp(&file(b))
                })
        });

        candidates
    }

    /// Candidates whose qualified name is, or ends with, `name`.
    ///
    /// A suffix match is what makes `Prompt.resolve_edge` find
    /// `dedupe_edges.Prompt.resolve_edge` without the user having to know the
    /// module prefix. The boundary check is what stops `Edge` from matching
    /// `ResolvedEdge` — a suffix that shares no scope is a different symbol.
    fn resolve_by_qualified_name(&self, name: &str) -> Vec<NodeId> {
        let exact: Vec<NodeId> = self
            .graph
            .node_indices()
            .filter(|idx| {
                self.graph
                    .node_weight(*idx)
                    .is_some_and(|n| n.qualified_name == name)
            })
            .collect();

        if !exact.is_empty() {
            return exact;
        }

        self.graph
            .node_indices()
            .filter(|idx| {
                self.graph
                    .node_weight(*idx)
                    .is_some_and(|n| qualified_name_ends_with(&n.qualified_name, name))
            })
            .collect()
    }

    /// The single node a user most likely meant by `name`.
    ///
    /// See [`resolve_symbol_ranked`](Self::resolve_symbol_ranked) for how ties
    /// are broken.
    pub fn resolve_symbol(&self, name: &str) -> Option<NodeId> {
        self.resolve_symbol_ranked(name).into_iter().next()
    }

    /// Finds all nodes in a file.
    pub fn find_by_file(&self, file: &str) -> Vec<&CodeNode> {
        self.file_index
            .get(file)
            .map(|indexes| {
                indexes
                    .iter()
                    .filter_map(|idx| self.graph.node_weight(*idx))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Searches for nodes whose name literally contains the query.
    ///
    /// Substring matching only: `search("login")` will not find
    /// `get_authenticated`. Use [`search_ranked`](Self::search_ranked) when the
    /// caller is asking about a concept rather than spelling a name.
    pub fn search(&self, query: &str) -> Vec<&CodeNode> {
        self.search_index
            .search(query)
            .iter()
            .filter_map(|id| self.graph.node_weight(*id))
            .collect()
    }

    /// Searches names, identifier tokens, related concepts, and documentation.
    ///
    /// Returns hits ordered strongest-first, each labelled with *how* it
    /// matched — so a caller can present an exact hit and a concept guess
    /// differently instead of blending them into one opaque ranking.
    ///
    /// ```ignore
    /// // Finds `get_authenticated`, `verifyJwt`, `hashPassword`, …
    /// for (node, hit) in graph.search_ranked("login") {
    ///     println!("{} ({})", node.name, hit.kind.label());
    /// }
    /// ```
    pub fn search_ranked(&self, query: &str) -> Vec<(&CodeNode, crate::search_index::SearchHit)> {
        self.search_index
            .search_ranked(query)
            .into_iter()
            .filter_map(|hit| self.graph.node_weight(hit.id).map(|node| (node, hit)))
            .collect()
    }

    /// Gets nodes that call the given node.
    ///
    /// Deduplicated by node — one entry per distinct caller, however many call
    /// sites it has. This used to walk `neighbors_directed` and re-look-up each
    /// pair with `find_edge`, which is wrong twice over on a `StableDiGraph`
    /// that allows parallel edges: `neighbors_directed` yields a neighbour once
    /// *per edge*, so a caller with three call sites was returned three times
    /// (measured on a real `scip-java` graph: 20,139 `Calls` edges collapse to
    /// 17,156 distinct pairs — 14.8% of every listing was a repeat), and
    /// `find_edge` returns only one edge for a pair, so a pair carrying both a
    /// `Calls` and e.g. an `Implements` edge either doubled up or lost the call
    /// entirely depending on which edge `find_edge` happened to hand back (32
    /// such pairs on that same graph). Now built on [`Self::related`], which
    /// sees every parallel edge via `edges_directed` and dedupes by `NodeId`.
    pub fn get_callers(&self, index: NodeId) -> Vec<&CodeNode> {
        self.related(index, &[EdgeKind::Calls], Direction::Incoming)
    }

    /// Types that implement or extend this one, directly.
    ///
    /// Incoming `Implements`/`Extends` edges. Kept separate from
    /// [`Self::get_callers`] because an implementor is not a caller: an interface
    /// method's implementations do not call it, they *are* it, and folding the
    /// two together would make blast radius double-count the hierarchy.
    pub fn implementors(&self, index: NodeId) -> Vec<&CodeNode> {
        self.related(index, INHERITANCE_KINDS, Direction::Incoming)
    }

    /// Types that implement or extend this one, at any depth.
    ///
    /// Interface → abstract base → concrete class is three levels and entirely
    /// ordinary, and the answer usually wanted is the concrete leaves. Returned
    /// breadth-first with each node's distance, so a caller can present the
    /// hierarchy rather than a flat list.
    ///
    /// `max_depth` bounds a hierarchy that is cyclic in a malformed index rather
    /// than trusting it to terminate.
    pub fn implementors_transitive(
        &self,
        index: NodeId,
        max_depth: usize,
    ) -> Vec<(&CodeNode, usize)> {
        self.related_transitive(index, INHERITANCE_KINDS, Direction::Incoming, max_depth)
    }

    /// Whether this graph contains any inheritance edge at all.
    ///
    /// The difference between "nothing implements this" and "this graph cannot
    /// answer that question". Tree-sitter emits no inheritance edges, and neither
    /// does every SCIP indexer — `rust-analyzer` emits no `is_implementation`
    /// relationships — so an empty result has two very different meanings and a
    /// caller must be able to tell them apart.
    pub fn has_inheritance_edges(&self) -> bool {
        self.has_edges_of_kind(INHERITANCE_KINDS)
    }

    /// Nodes connected to `index` by any edge whose kind is in `kinds`, in `direction`.
    ///
    /// Takes a slice rather than a single kind because some relationships are
    /// more than one `EdgeKind` — inheritance is `Implements` *and* `Extends` —
    /// and a single-kind signature would need a parallel "one of several kinds"
    /// implementation immediately.
    ///
    /// Traverses with `edges_directed` rather than `neighbors_directed` +
    /// `find_edge`, unlike the older `get_callers`/`get_callees` pattern.
    /// `StableDiGraph::add_edge` allows parallel edges, and a SCIP graph really
    /// does carry both e.g. a `Calls` and a `UsesType` edge between the same two
    /// nodes; `find_edge` returns only one edge for a pair, so whichever kind it
    /// does not return becomes invisible. `edges_directed` yields every edge, so
    /// none of them are lost.
    ///
    /// Deduplicates the returned nodes by [`NodeId`], preserving first-seen
    /// order — two parallel edges of the same kind must not yield the same node
    /// twice.
    pub fn related(
        &self,
        index: NodeId,
        kinds: &[EdgeKind],
        direction: Direction,
    ) -> Vec<&CodeNode> {
        let mut seen: std::collections::HashSet<NodeId> = std::collections::HashSet::new();
        let mut out = Vec::new();

        for edge in self.graph.edges_directed(index, direction) {
            if !kinds.contains(&edge.weight().kind) {
                continue;
            }
            let neighbor = match direction {
                Direction::Incoming => edge.source(),
                Direction::Outgoing => edge.target(),
            };
            if !seen.insert(neighbor) {
                continue;
            }
            if let Some(node) = self.graph.node_weight(neighbor) {
                out.push(node);
            }
        }

        out
    }

    /// Nodes connected to `index` by edges of `kinds`, at any depth in `direction`.
    ///
    /// Mirrors [`Self::implementors_transitive`] exactly: breadth-first, each
    /// node paired with its distance starting at 1, `max_depth` bounds a
    /// relationship graph that may be cyclic in a malformed index, and the
    /// start node is marked seen up front so it never appears in the output.
    pub fn related_transitive(
        &self,
        index: NodeId,
        kinds: &[EdgeKind],
        direction: Direction,
        max_depth: usize,
    ) -> Vec<(&CodeNode, usize)> {
        let mut seen: std::collections::HashSet<NodeId> = std::collections::HashSet::new();
        seen.insert(index);

        let mut frontier = vec![index];
        let mut out = Vec::new();

        for depth in 1..=max_depth {
            let mut next = Vec::new();
            for current in &frontier {
                for edge in self.graph.edges_directed(*current, direction) {
                    if !kinds.contains(&edge.weight().kind) {
                        continue;
                    }
                    let neighbor = match direction {
                        Direction::Incoming => edge.source(),
                        Direction::Outgoing => edge.target(),
                    };
                    if !seen.insert(neighbor) {
                        continue;
                    }
                    if let Some(node) = self.graph.node_weight(neighbor) {
                        out.push((node, depth));
                        next.push(neighbor);
                    }
                }
            }
            if next.is_empty() {
                break;
            }
            frontier = next;
        }

        out
    }

    /// Groups a node's edges in `direction` by kind.
    ///
    /// Kinds with no edges are omitted rather than inserted as empty vectors.
    /// Nodes are deduplicated within each kind the same way [`Self::related`]
    /// dedupes: parallel edges of the same kind must not repeat a node.
    pub fn relationships_by_kind(
        &self,
        index: NodeId,
        direction: Direction,
    ) -> BTreeMap<EdgeKind, Vec<&CodeNode>> {
        let mut by_kind: BTreeMap<EdgeKind, Vec<&CodeNode>> = BTreeMap::new();
        let mut seen: std::collections::HashSet<(EdgeKind, NodeId)> =
            std::collections::HashSet::new();

        for edge in self.graph.edges_directed(index, direction) {
            let kind = edge.weight().kind;
            let neighbor = match direction {
                Direction::Incoming => edge.source(),
                Direction::Outgoing => edge.target(),
            };
            if !seen.insert((kind, neighbor)) {
                continue;
            }
            if let Some(node) = self.graph.node_weight(neighbor) {
                by_kind.entry(kind).or_default().push(node);
            }
        }

        by_kind
    }

    /// Whether the graph contains any edge whose kind is in `kinds`.
    ///
    /// Same shape as [`Self::has_inheritance_edges`]: an empty result from
    /// [`Self::related`] is ambiguous between "none exist" and "this indexer
    /// never emits that kind", and callers need to tell the two apart.
    pub fn has_edges_of_kind(&self, kinds: &[EdgeKind]) -> bool {
        (&self.graph)
            .edge_references()
            .any(|edge| kinds.contains(&edge.weight().kind))
    }

    /// Gets nodes that this node calls.
    ///
    /// Same fix, same reasoning, as [`Self::get_callers`]: deduplicated by
    /// node via [`Self::related`] rather than walking `neighbors_directed` and
    /// re-resolving each pair with `find_edge`, which duplicated a callee with
    /// multiple call sites and could lose a call entirely when the same pair
    /// also carried a non-`Calls` edge.
    pub fn get_callees(&self, index: NodeId) -> Vec<&CodeNode> {
        self.related(index, &[EdgeKind::Calls], Direction::Outgoing)
    }

    /// Gets all nodes that depend on the given node (directly or transitively).
    pub fn get_dependents(&self, index: NodeId, max_depth: usize) -> Vec<(NodeId, usize)> {
        let mut result = Vec::new();
        let mut visited = std::collections::HashSet::new();
        let mut queue = vec![(index, 0usize)];

        while let Some((current, depth)) = queue.pop() {
            if depth > max_depth || visited.contains(&current) {
                continue;
            }
            visited.insert(current);

            if current != index {
                result.push((current, depth));
            }

            // Get incoming edges (callers)
            for neighbor in self
                .graph
                .neighbors_directed(current, petgraph::Direction::Incoming)
            {
                if !visited.contains(&neighbor) {
                    queue.push((neighbor, depth + 1));
                }
            }
        }

        result
    }

    /// Removes all nodes from a file. Used for incremental updates.
    pub fn remove_file(&mut self, file: &str) {
        if let Some(indexes) = self.file_index.remove(file) {
            for index in indexes {
                if let Some(node) = self.graph.node_weight(index) {
                    // Remove from name index
                    let name = node.name.clone();
                    if let Some(name_list) = self.name_index.get_mut(&name) {
                        name_list.retain(|&idx| idx != index);
                    }
                    // Remove from id index
                    self.id_index.remove(&node.id);
                    // Remove from search index
                    self.search_index.remove(&name, index);
                }
                self.graph.remove_node(index);
            }
        }
    }

    /// Centrality as a percentile rank in `[0.0, 1.0]`.
    ///
    /// `0.9` means "more central than 90% of the nodes in this repository",
    /// and carries that meaning in every repository — so a threshold written
    /// against it behaves the same on a god-object monolith and a flat service.
    /// For the underlying PageRank mass see [`centrality_raw`](Self::centrality_raw).
    pub fn centrality(&self, index: NodeId) -> f64 {
        self.centrality.get(&index).copied().unwrap_or(0.0)
    }

    /// Raw PageRank mass for a node. Sums to ~1.0 across the graph.
    pub fn centrality_raw(&self, index: NodeId) -> f64 {
        self.centrality_raw.get(&index).copied().unwrap_or(0.0)
    }

    /// Iterates every edge weight, for density and confidence reporting.
    pub fn edge_weights(&self) -> impl Iterator<Item = &Edge> {
        self.graph.edge_weights()
    }

    /// Counts edges at or above [`Edge::CONFIDENT_THRESHOLD`].
    ///
    /// The gap between this and [`edge_count`](Self::edge_count) is how much of
    /// the graph rests on inference rather than proof — worth surfacing in any
    /// report that claims to describe what a change reaches.
    pub fn confident_edge_count(&self) -> usize {
        self.graph
            .edge_weights()
            .filter(|edge| edge.is_confident())
            .count()
    }

    /// Sets percentile centrality scores.
    ///
    /// Prefer [`set_centrality_scores`](Self::set_centrality_scores), which
    /// also stores the raw scores a warm start needs.
    pub fn set_centrality(&mut self, scores: HashMap<NodeId, f64>) {
        self.centrality = scores;
    }

    /// Stores both the raw and percentile forms from a computation.
    pub fn set_centrality_scores(&mut self, scores: crate::ranking::CentralityScores) {
        let (raw, percentile) = scores.into_parts();
        self.centrality_raw = raw;
        self.centrality = percentile;
    }

    /// Raw score map, for warm-starting a recompute.
    ///
    /// Falls back to the percentile map when raw scores are absent — a graph
    /// deserialized from a cache written before raw scores were stored. The
    /// warm-start rescale in [`crate::ranking`] tolerates any scalar multiple
    /// of the fixed point, so this degrades convergence speed, not correctness.
    pub fn centrality_map(&self) -> &HashMap<NodeId, f64> {
        if self.centrality_raw.is_empty() {
            &self.centrality
        } else {
            &self.centrality_raw
        }
    }

    /// Returns the number of nodes.
    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    /// Returns the number of edges.
    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }

    /// Iterates over all nodes.
    pub fn nodes(&self) -> impl Iterator<Item = &CodeNode> {
        self.graph.node_weights()
    }

    /// Iterates over all edges.
    pub fn edges(&self) -> impl Iterator<Item = &Edge> {
        self.graph.edge_weights()
    }

    /// Returns all edges with source and target IDs for export.
    pub fn export_edges(&self) -> Vec<GraphEdge> {
        (&self.graph)
            .edge_references()
            .filter_map(|edge_ref| {
                let source = self.graph.node_weight(edge_ref.source())?.id.clone();
                let target = self.graph.node_weight(edge_ref.target())?.id.clone();
                let weight = edge_ref.weight(); // &Edge
                Some(GraphEdge {
                    source,
                    target,
                    kind: weight.kind,
                })
            })
            .collect()
    }

    /// Like [`Self::export_edges`], but carries confidence/file/line through
    /// for `arbor export` — the bulk dump, where dropping that detail is what
    /// forced people to decode `graph.json`'s petgraph serialisation by hand
    /// instead.
    pub fn export_edges_detailed(&self) -> Vec<ExportEdge> {
        (&self.graph)
            .edge_references()
            .filter_map(|edge_ref| {
                let source = self.graph.node_weight(edge_ref.source())?.id.clone();
                let target = self.graph.node_weight(edge_ref.target())?.id.clone();
                let weight = edge_ref.weight(); // &Edge
                Some(ExportEdge {
                    source,
                    target,
                    kind: weight.kind,
                    confidence: weight.confidence,
                    file: weight.file.clone(),
                    line: weight.line,
                })
            })
            .collect()
    }

    /// Iterates over all node indexes.
    pub fn node_indexes(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.graph.node_indices()
    }

    /// Finds the shortest path between two nodes.
    pub fn find_path(&self, from: NodeId, to: NodeId) -> Option<Vec<&CodeNode>> {
        let path_indices = petgraph::algo::astar(
            &self.graph,
            from,
            |finish| finish == to,
            |_| 1, // weight of 1 for all edges (BFS-like)
            |_| 0, // heuristic
        )?;

        Some(
            path_indices
                .1
                .into_iter()
                .filter_map(|idx| self.graph.node_weight(idx))
                .collect(),
        )
    }

    /// Gets the node index for a string ID.
    pub fn get_index(&self, id: &str) -> Option<NodeId> {
        self.id_index.get(id).copied()
    }

    /// Returns all nodes detected as production entry points.
    pub fn list_entry_points(&self) -> Vec<&CodeNode> {
        use crate::heuristics::HeuristicsMatcher;
        self.graph
            .node_weights()
            .filter(|n| HeuristicsMatcher::is_likely_entry_point(n))
            .collect()
    }

    /// Returns all nodes in a file and the call edges between them.
    /// Edges returned as (caller_name, callee_name, edge_kind_debug_str) triples.
    pub fn nodes_in_file_with_edges(
        &self,
        file: &str,
    ) -> (Vec<&CodeNode>, Vec<(String, String, String)>) {
        // Exact key first; fall back to a separator-insensitive match.
        //
        // `file_index` is keyed on the path exactly as the parser recorded it.
        // Callers build lookup paths by joining a project root with a
        // forward-slash relative path, which on Windows yields a mixed
        // `C:\root\src/lib.rs` that never equals the stored `C:\root\src\lib.rs`.
        let node_ids: std::collections::HashSet<NodeId> = match self.file_index.get(file) {
            Some(ids) => ids.iter().copied().collect(),
            None => {
                let wanted = normalize_separators(file);
                self.file_index
                    .iter()
                    .filter(|(stored, _)| normalize_separators(stored) == wanted)
                    .flat_map(|(_, ids)| ids.iter().copied())
                    .collect()
            }
        };

        let nodes: Vec<&CodeNode> = node_ids
            .iter()
            .filter_map(|&id| self.graph.node_weight(id))
            .collect();

        let mut edges = Vec::new();
        for &from in &node_ids {
            for edge_ref in self
                .graph
                .edges_directed(from, petgraph::Direction::Outgoing)
            {
                let to = edge_ref.target();
                if node_ids.contains(&to) {
                    if let (Some(from_node), Some(to_node)) =
                        (self.graph.node_weight(from), self.graph.node_weight(to))
                    {
                        edges.push((
                            from_node.name.clone(),
                            to_node.name.clone(),
                            format!("{:?}", edge_ref.weight().kind),
                        ));
                    }
                }
            }
        }
        (nodes, edges)
    }
}

/// Graph statistics for the info endpoint.
#[derive(Debug, Serialize, Deserialize)]
pub struct GraphStats {
    pub node_count: usize,
    pub edge_count: usize,
    pub files: usize,
}

impl ArborGraph {
    /// Returns graph statistics.
    pub fn stats(&self) -> GraphStats {
        GraphStats {
            node_count: self.node_count(),
            edge_count: self.edge_count(),
            files: self.file_index.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edge::{Edge, EdgeKind};
    use arbor_core::{CodeNode, NodeKind};

    fn make_node(name: &str, file: &str) -> CodeNode {
        CodeNode::new(name, name, NodeKind::Function, file)
    }

    #[test]
    fn test_graph_new_is_empty() {
        let g = ArborGraph::new();
        assert_eq!(g.node_count(), 0);
        assert_eq!(g.edge_count(), 0);
        assert!(g.nodes().next().is_none());
    }

    #[test]
    fn test_graph_add_and_get_node() {
        let mut g = ArborGraph::new();
        let node = make_node("foo", "main.rs");
        let id = g.add_node(node.clone());
        assert_eq!(g.node_count(), 1);

        let got = g.get(id).unwrap();
        assert_eq!(got.name, "foo");
    }

    #[test]
    fn test_graph_find_by_name() {
        let mut g = ArborGraph::new();
        g.add_node(make_node("alpha", "a.rs"));
        g.add_node(make_node("beta", "b.rs"));

        let found = g.find_by_name("alpha");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "alpha");

        let not_found = g.find_by_name("gamma");
        assert!(not_found.is_empty());
    }

    #[test]
    fn test_graph_find_by_file() {
        let mut g = ArborGraph::new();
        g.add_node(make_node("foo", "main.rs"));
        g.add_node(make_node("bar", "main.rs"));
        g.add_node(make_node("baz", "other.rs"));

        let main_nodes = g.find_by_file("main.rs");
        assert_eq!(main_nodes.len(), 2);

        let other_nodes = g.find_by_file("other.rs");
        assert_eq!(other_nodes.len(), 1);

        let empty = g.find_by_file("nonexistent.rs");
        assert!(empty.is_empty());
    }

    #[test]
    fn test_graph_search_substring() {
        let mut g = ArborGraph::new();
        g.add_node(make_node("validate_user", "a.rs"));
        g.add_node(make_node("validate_email", "b.rs"));
        g.add_node(make_node("send_email", "c.rs"));

        let results = g.search("validate");
        assert_eq!(results.len(), 2);
        assert!(results.iter().any(|n| n.name == "validate_user"));
        assert!(results.iter().any(|n| n.name == "validate_email"));
    }

    #[test]
    fn test_graph_callers_callees() {
        let mut g = ArborGraph::new();
        let a = g.add_node(make_node("caller", "a.rs"));
        let b = g.add_node(make_node("callee", "b.rs"));
        g.add_edge(a, b, Edge::new(EdgeKind::Calls));

        let callees = g.get_callees(a);
        assert_eq!(callees.len(), 1);
        assert_eq!(callees[0].name, "callee");

        let callers = g.get_callers(b);
        assert_eq!(callers.len(), 1);
        assert_eq!(callers[0].name, "caller");

        // No callers/callees for disconnected nodes
        assert!(g.get_callers(a).is_empty());
        assert!(g.get_callees(b).is_empty());
    }

    #[test]
    fn parallel_calls_edges_yield_the_caller_and_callee_once() {
        // Two call sites from the same caller to the same callee — the
        // `StableDiGraph` allows the parallel edge, and `get_callers`/
        // `get_callees` must not report the pair twice just because it has
        // two edges.
        let mut g = ArborGraph::new();
        let a = g.add_node(make_node("caller", "a.rs"));
        let b = g.add_node(make_node("callee", "b.rs"));
        g.add_edge(a, b, Edge::new(EdgeKind::Calls));
        g.add_edge(a, b, Edge::new(EdgeKind::Calls));

        let callers = g.get_callers(b);
        assert_eq!(callers.len(), 1, "two parallel Calls edges, one caller");
        assert_eq!(callers[0].name, "caller");

        let callees = g.get_callees(a);
        assert_eq!(callees.len(), 1, "two parallel Calls edges, one callee");
        assert_eq!(callees[0].name, "callee");
    }

    #[test]
    fn a_pair_with_both_calls_and_implements_loses_neither() {
        // The 32-pair case from the real graph: a `Calls` edge and an
        // `Implements` edge between the same two nodes. The old
        // `find_edge`-based lookup returned only one of them per pair, so
        // this either duplicated the call or silently dropped it depending on
        // which edge `find_edge` happened to hand back. `get_callers` and
        // `implementors` must each see their own edge, independent of the other.
        let mut g = ArborGraph::new();
        let base = g.add_node(make_node("Base", "base.rs"));
        let derived = g.add_node(make_node("Derived", "derived.rs"));
        g.add_edge(derived, base, Edge::new(EdgeKind::Calls));
        g.add_edge(derived, base, Edge::new(EdgeKind::Implements));

        let callers = g.get_callers(base);
        assert_eq!(callers.len(), 1, "the Calls edge must not be hidden");
        assert_eq!(callers[0].name, "Derived");

        let implementors = g.implementors(base);
        assert_eq!(
            implementors.len(),
            1,
            "the Implements edge must not be hidden"
        );
        assert_eq!(implementors[0].name, "Derived");
    }

    #[test]
    fn test_export_edges_detailed_round_trips_confidence() {
        let mut g = ArborGraph::new();
        let a = g.add_node(make_node("caller", "a.rs"));
        let b = g.add_node(make_node("callee", "b.rs"));
        let c = g.add_node(make_node("base", "c.rs"));

        let id_a = g.get(a).unwrap().id.clone();
        let id_b = g.get(b).unwrap().id.clone();
        let id_c = g.get(c).unwrap().id.clone();

        g.add_edge(
            a,
            b,
            Edge::with_location(EdgeKind::Calls, "a.rs", 7).with_confidence(0.4),
        );
        g.add_edge(b, c, Edge::new(EdgeKind::Implements));

        let exported = g.export_edges_detailed();
        assert_eq!(exported.len(), 2);

        let calls_edge = exported
            .iter()
            .find(|e| e.kind == EdgeKind::Calls)
            .expect("calls edge present");
        assert_eq!(calls_edge.source, id_a);
        assert_eq!(calls_edge.target, id_b);
        assert_eq!(calls_edge.confidence, 0.4);
        assert_eq!(calls_edge.file, Some("a.rs".to_string()));
        assert_eq!(calls_edge.line, Some(7));

        let implements_edge = exported
            .iter()
            .find(|e| e.kind == EdgeKind::Implements)
            .expect("implements edge present");
        assert_eq!(implements_edge.source, id_b);
        assert_eq!(implements_edge.target, id_c);
        assert_eq!(implements_edge.confidence, 1.0);
    }

    #[test]
    fn test_graph_get_dependents() {
        // a -> b -> c
        let mut g = ArborGraph::new();
        let a = g.add_node(make_node("a", "a.rs"));
        let b = g.add_node(make_node("b", "b.rs"));
        let c = g.add_node(make_node("c", "c.rs"));
        g.add_edge(a, b, Edge::new(EdgeKind::Calls));
        g.add_edge(b, c, Edge::new(EdgeKind::Calls));

        // Dependents of c at depth 2 should include a and b
        let deps = g.get_dependents(c, 2);
        assert!(deps.iter().any(|(idx, _)| g.get(*idx).unwrap().name == "b"));
        assert!(deps.iter().any(|(idx, _)| g.get(*idx).unwrap().name == "a"));
    }

    #[test]
    fn test_graph_remove_file_cleanup() {
        let mut g = ArborGraph::new();
        g.add_node(make_node("foo", "remove_me.rs"));
        g.add_node(make_node("bar", "remove_me.rs"));
        g.add_node(make_node("keep", "keep.rs"));

        assert_eq!(g.node_count(), 3);

        g.remove_file("remove_me.rs");

        // Nodes from removed file are gone
        assert!(g.find_by_name("foo").is_empty());
        assert!(g.find_by_name("bar").is_empty());
        // Node from other file remains
        assert_eq!(g.find_by_name("keep").len(), 1);
        assert!(g.find_by_file("remove_me.rs").is_empty());
    }

    #[test]
    fn test_graph_find_path() {
        // a -> b -> c
        let mut g = ArborGraph::new();
        let a = g.add_node(make_node("start", "a.rs"));
        let b = g.add_node(make_node("middle", "b.rs"));
        let c = g.add_node(make_node("end", "c.rs"));
        g.add_edge(a, b, Edge::new(EdgeKind::Calls));
        g.add_edge(b, c, Edge::new(EdgeKind::Calls));

        let path = g.find_path(a, c).unwrap();
        assert_eq!(path.len(), 3);
        assert_eq!(path[0].name, "start");
        assert_eq!(path[1].name, "middle");
        assert_eq!(path[2].name, "end");
    }

    #[test]
    fn test_graph_find_path_no_connection() {
        let mut g = ArborGraph::new();
        let a = g.add_node(make_node("island_a", "a.rs"));
        let b = g.add_node(make_node("island_b", "b.rs"));

        // No edges → no path
        assert!(g.find_path(a, b).is_none());
    }

    #[test]
    fn test_graph_export_edges() {
        let mut g = ArborGraph::new();
        let a = g.add_node(make_node("a", "a.rs"));
        let b = g.add_node(make_node("b", "b.rs"));
        g.add_edge(a, b, Edge::new(EdgeKind::Calls));

        let exported = g.export_edges();
        assert_eq!(exported.len(), 1);
        assert_eq!(exported[0].kind, EdgeKind::Calls);
    }

    #[test]
    fn test_graph_stats() {
        let mut g = ArborGraph::new();
        g.add_node(make_node("a", "x.rs"));
        g.add_node(make_node("b", "y.rs"));

        let stats = g.stats();
        assert_eq!(stats.node_count, 2);
        assert_eq!(stats.edge_count, 0);
        assert_eq!(stats.files, 2);
    }

    #[test]
    fn test_graph_get_index_and_get_by_id() {
        let mut g = ArborGraph::new();
        let node = make_node("lookup_me", "test.rs");
        let node_id_str = node.id.clone();
        let idx = g.add_node(node);

        assert_eq!(g.get_index(&node_id_str), Some(idx));
        assert!(g.get_by_id(&node_id_str).is_some());
        assert!(g.get_index("nonexistent").is_none());
        assert!(g.get_by_id("nonexistent").is_none());
    }

    #[test]
    fn test_graph_centrality_default_zero() {
        let mut g = ArborGraph::new();
        let idx = g.add_node(make_node("a", "a.rs"));
        assert_eq!(g.centrality(idx), 0.0);
    }

    #[test]
    fn test_graph_set_centrality() {
        let mut g = ArborGraph::new();
        let idx = g.add_node(make_node("a", "a.rs"));

        let mut scores = HashMap::new();
        scores.insert(idx, 0.75);
        g.set_centrality(scores);

        assert!((g.centrality(idx) - 0.75).abs() < f64::EPSILON);
    }
}

#[cfg(test)]
mod new_query_tests {
    use super::*;
    use crate::edge::{Edge, EdgeKind};
    use arbor_core::{CodeNode, NodeKind};

    fn make_node(name: &str, kind: NodeKind, file: &str) -> CodeNode {
        CodeNode::new(name, format!("{}::{}", file, name), kind, file)
    }

    #[test]
    fn test_list_entry_points_returns_main() {
        let mut g = ArborGraph::new();
        g.add_node(make_node("main", NodeKind::Function, "src/main.rs"));
        g.add_node(make_node("helper", NodeKind::Function, "src/util.rs"));
        let eps = g.list_entry_points();
        assert!(
            eps.iter().any(|n| n.name == "main"),
            "main must be an entry point"
        );
        assert!(
            !eps.iter().any(|n| n.name == "helper"),
            "helper must not be an entry point"
        );
    }

    #[test]
    fn test_nodes_in_file_with_edges_returns_edges() {
        let mut g = ArborGraph::new();
        let a = g.add_node(make_node("foo", NodeKind::Function, "src/a.rs"));
        let b = g.add_node(make_node("bar", NodeKind::Function, "src/a.rs"));
        let _c = g.add_node(make_node("baz", NodeKind::Function, "src/b.rs"));
        g.add_edge(a, b, Edge::new(EdgeKind::Calls));
        let (nodes, edges) = g.nodes_in_file_with_edges("src/a.rs");
        assert_eq!(nodes.len(), 2);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].0, "foo");
        assert_eq!(edges[0].1, "bar");
    }

    #[test]
    fn nodes_in_file_lookup_is_separator_insensitive() {
        // Callers join a project root with a forward-slash relative path,
        // which on Windows yields `C:\root\src/lib.rs` while the parser
        // recorded `C:\root\src\lib.rs`. Exact-key lookup missed, and
        // `file-graph` reported "No symbols found" for a file full of symbols.
        let mut g = ArborGraph::new();
        g.add_node(make_node(
            "helper",
            NodeKind::Function,
            r"C:/root/src\lib.rs",
        ));

        let (backslash, _) = g.nodes_in_file_with_edges(r"C:/root/src\lib.rs");
        assert_eq!(backslash.len(), 1, "exact key must still work");

        let (forward, _) = g.nodes_in_file_with_edges("C:/root/src/lib.rs");
        assert_eq!(forward.len(), 1, "separator spelling must not matter");
    }

    #[test]
    fn test_nodes_in_file_with_edges_excludes_cross_file_edges() {
        use crate::edge::{Edge, EdgeKind};
        let mut g = ArborGraph::new();
        let a = g.add_node(make_node("foo", NodeKind::Function, "src/a.rs"));
        let c = g.add_node(make_node("baz", NodeKind::Function, "src/b.rs"));
        // Edge from a.rs to b.rs — should NOT appear in get_file_graph for a.rs
        g.add_edge(a, c, Edge::new(EdgeKind::Calls));
        let (nodes, edges) = g.nodes_in_file_with_edges("src/a.rs");
        assert_eq!(nodes.len(), 1); // only foo
        assert_eq!(edges.len(), 0); // cross-file edge excluded
    }
}

/// Feeds a node's supporting text into the search index.
///
/// Docstrings, signatures, and qualified names are already parsed into every
/// `CodeNode` and were previously ignored at search time — so a function
/// documented as "validates the user's login credentials" was unreachable by a
/// search for "login". The file path is included too: `src/auth/session.ts`
/// says as much about a symbol as its name often does.
fn index_node_documentation(index: &mut SearchIndex, node: &CodeNode, id: NodeId) {
    if let Some(doc) = &node.docstring {
        index.insert_documentation(doc, id);
    }
    if let Some(sig) = &node.signature {
        index.insert_documentation(sig, id);
    }
    index.insert_documentation(&node.file, id);
    if node.qualified_name != node.name {
        index.insert_documentation(&node.qualified_name, id);
    }
}

/// Normalizes path separators for comparison.
///
/// Windows accepts both `/` and `\`, so the same file can be spelled either
/// way depending on whether the path came from a directory walk or from
/// joining a user-supplied relative path.
fn normalize_separators(path: &str) -> String {
    path.replace('\\', "/")
}

/// Whether `qualified_name` ends with `suffix` at a scope boundary.
///
/// `Prompt.resolve_edge` matches `dedupe_edges.Prompt.resolve_edge`;
/// `Edge` does not match `ResolvedEdge`. Both `.` and `::` are accepted so the
/// same check serves Python, Java and TypeScript names alongside Rust and C++
/// ones — the separator is a language's spelling, not a different concept.
fn qualified_name_ends_with(qualified_name: &str, suffix: &str) -> bool {
    let Some(head) = qualified_name.strip_suffix(suffix) else {
        return false;
    };

    head.ends_with('.') || head.ends_with("::") || head.ends_with('#') || head.ends_with('/')
}

/// Kinds a user means when they ask "who calls this".
///
/// Used only to break a tie between candidates that are otherwise
/// indistinguishable, so it deliberately does not include types.
fn is_callable_kind(kind: arbor_core::NodeKind) -> bool {
    use arbor_core::NodeKind;
    matches!(
        kind,
        NodeKind::Function | NodeKind::Method | NodeKind::Constructor
    )
}

#[cfg(test)]
mod resolution_tests {
    use super::*;
    use arbor_core::NodeKind;

    fn node(name: &str, qualified: &str, kind: NodeKind, file: &str, line: u32) -> CodeNode {
        CodeNode::new(name, qualified, kind, file).with_lines(line, line)
    }

    /// The zep_config case: `resolve_edge` is two TypedDict fields and one
    /// module-level function, none of them connected.
    fn ambiguous_graph() -> ArborGraph {
        let mut graph = ArborGraph::new();
        graph.add_node(node(
            "resolve_edge",
            "dedupe_edges.Prompt.resolve_edge",
            NodeKind::Field,
            "dedupe_edges.py",
            36,
        ));
        graph.add_node(node(
            "resolve_edge",
            "dedupe_edges.Versions.resolve_edge",
            NodeKind::Field,
            "dedupe_edges.py",
            40,
        ));
        graph.add_node(node(
            "resolve_edge",
            "dedupe_edges.resolve_edge",
            NodeKind::Function,
            "dedupe_edges.py",
            43,
        ));
        graph
    }

    #[test]
    fn a_qualified_name_resolves() {
        // The ambiguity note prints these; passing one back must work.
        let graph = ambiguous_graph();
        let found = graph.resolve_symbol_ranked("dedupe_edges.Versions.resolve_edge");
        assert_eq!(found.len(), 1);
        assert_eq!(
            graph.get(found[0]).unwrap().kind,
            NodeKind::Field,
            "must pick the exact node named, not the most appealing one"
        );
    }

    #[test]
    fn a_partial_qualified_name_resolves_at_a_scope_boundary() {
        let graph = ambiguous_graph();
        let found = graph.resolve_symbol_ranked("Prompt.resolve_edge");
        assert_eq!(found.len(), 1);
        assert_eq!(
            graph.get(found[0]).unwrap().qualified_name,
            "dedupe_edges.Prompt.resolve_edge"
        );
    }

    #[test]
    fn a_suffix_that_is_not_a_scope_boundary_does_not_match() {
        let mut graph = ArborGraph::new();
        graph.add_node(node(
            "ResolvedEdge",
            "edges.ResolvedEdge",
            NodeKind::Class,
            "edges.py",
            1,
        ));
        assert!(graph.resolve_symbol_ranked("Edge").is_empty());
    }

    #[test]
    fn rust_paths_resolve_too() {
        let mut graph = ArborGraph::new();
        graph.add_node(node(
            "add_edge",
            "graph::ArborGraph::add_edge",
            NodeKind::Method,
            "graph.rs",
            1,
        ));
        assert_eq!(
            graph
                .resolve_symbol_ranked("graph::ArborGraph::add_edge")
                .len(),
            1
        );
        assert_eq!(
            graph.resolve_symbol_ranked("ArborGraph::add_edge").len(),
            1,
            "a `::` boundary counts the same as a `.` one"
        );
    }

    #[test]
    fn an_unconnected_tie_prefers_the_callable() {
        // Previously this picked a TypedDict field over the function of the same
        // name, then reported it isolated — which it genuinely is.
        let graph = ambiguous_graph();
        let ranked = graph.resolve_symbol_ranked("resolve_edge");
        assert_eq!(ranked.len(), 3);
        assert_eq!(
            graph.get(ranked[0]).unwrap().kind,
            NodeKind::Function,
            "a field is never what 'who calls resolve_edge' meant"
        );
    }

    #[test]
    fn connectedness_still_outranks_callability() {
        // A field with real edges beats an unconnected function: the tie-break
        // must only apply when there is actually a tie.
        let mut graph = ArborGraph::new();
        let field = graph.add_node(node("handler", "App.handler", NodeKind::Field, "a.py", 1));
        graph.add_node(node("handler", "b.handler", NodeKind::Function, "b.py", 1));
        let caller = graph.add_node(node("main", "main", NodeKind::Function, "c.py", 1));
        graph.add_edge(caller, field, Edge::new(EdgeKind::Calls));

        let ranked = graph.resolve_symbol_ranked("handler");
        assert_eq!(graph.get(ranked[0]).unwrap().qualified_name, "App.handler");
    }
}

#[cfg(test)]
mod inheritance_tests {
    use super::*;
    use arbor_core::NodeKind;

    fn node(qualified: &str, kind: NodeKind) -> CodeNode {
        let name = qualified.rsplit('.').next().unwrap_or(qualified);
        CodeNode::new(name, qualified, kind, "x.java").with_lines(1, 1)
    }

    /// Interface → abstract base → two concrete classes.
    fn hierarchy() -> (ArborGraph, NodeId) {
        let mut graph = ArborGraph::new();
        let gateway = graph.add_node(node("Gateway", NodeKind::Interface));
        let base = graph.add_node(node("AbstractGateway", NodeKind::Class));
        let stripe = graph.add_node(node("StripeGateway", NodeKind::Class));
        let paypal = graph.add_node(node("PaypalGateway", NodeKind::Class));

        graph.add_edge(base, gateway, Edge::new(EdgeKind::Implements));
        graph.add_edge(stripe, base, Edge::new(EdgeKind::Extends));
        graph.add_edge(paypal, base, Edge::new(EdgeKind::Extends));
        // A call edge must not be mistaken for a hierarchy edge.
        let checkout = graph.add_node(node("Checkout", NodeKind::Class));
        graph.add_edge(checkout, gateway, Edge::new(EdgeKind::Calls));

        (graph, gateway)
    }

    #[test]
    fn direct_implementors_exclude_callers() {
        let (graph, gateway) = hierarchy();
        let names: Vec<&str> = graph
            .implementors(gateway)
            .iter()
            .map(|n| n.qualified_name.as_str())
            .collect();
        assert_eq!(names, vec!["AbstractGateway"]);
    }

    #[test]
    fn transitive_implementors_reach_the_concrete_leaves() {
        let (graph, gateway) = hierarchy();
        let mut found: Vec<(String, usize)> = graph
            .implementors_transitive(gateway, 4)
            .into_iter()
            .map(|(n, d)| (n.qualified_name.clone(), d))
            .collect();
        found.sort();

        assert_eq!(
            found,
            vec![
                ("AbstractGateway".to_string(), 1),
                ("PaypalGateway".to_string(), 2),
                ("StripeGateway".to_string(), 2),
            ]
        );
    }

    #[test]
    fn depth_is_bounded() {
        let (graph, gateway) = hierarchy();
        let found = graph.implementors_transitive(gateway, 1);
        assert_eq!(found.len(), 1, "depth 1 is the direct implementors only");
    }

    #[test]
    fn a_cycle_terminates() {
        // A malformed index can claim A implements B and B implements A.
        let mut graph = ArborGraph::new();
        let a = graph.add_node(node("A", NodeKind::Class));
        let b = graph.add_node(node("B", NodeKind::Class));
        graph.add_edge(a, b, Edge::new(EdgeKind::Implements));
        graph.add_edge(b, a, Edge::new(EdgeKind::Implements));

        let found = graph.implementors_transitive(a, 16);
        assert_eq!(found.len(), 1, "each node is visited once");
    }

    #[test]
    fn a_graph_without_hierarchy_says_so() {
        // The distinction between "nothing implements this" and "this graph
        // cannot answer that".
        let mut graph = ArborGraph::new();
        let a = graph.add_node(node("A", NodeKind::Class));
        let b = graph.add_node(node("B", NodeKind::Class));
        graph.add_edge(b, a, Edge::new(EdgeKind::Calls));

        assert!(!graph.has_inheritance_edges());
        assert!(graph.implementors(a).is_empty());

        let (with_hierarchy, _) = hierarchy();
        assert!(with_hierarchy.has_inheritance_edges());
    }
}

#[cfg(test)]
mod related_tests {
    use super::*;
    use arbor_core::NodeKind;

    fn node(name: &str) -> CodeNode {
        CodeNode::new(name, name, NodeKind::Class, "x.java").with_lines(1, 1)
    }

    #[test]
    fn parallel_edges_of_different_kinds_are_both_visible() {
        // The case `find_edge` gets wrong: two edges of different kinds
        // between the same pair, where `find_edge` returns only one of them.
        let mut graph = ArborGraph::new();
        let a = graph.add_node(node("A"));
        let b = graph.add_node(node("B"));
        graph.add_edge(a, b, Edge::new(EdgeKind::Calls));
        graph.add_edge(a, b, Edge::new(EdgeKind::UsesType));

        let via_uses_type = graph.related(b, &[EdgeKind::UsesType], Direction::Incoming);
        assert_eq!(via_uses_type.len(), 1);
        assert_eq!(via_uses_type[0].name, "A");

        let via_calls = graph.related(b, &[EdgeKind::Calls], Direction::Incoming);
        assert_eq!(via_calls.len(), 1);
        assert_eq!(via_calls[0].name, "A");
    }

    #[test]
    fn parallel_edges_of_the_same_kind_dedupe_to_one_node() {
        let mut graph = ArborGraph::new();
        let a = graph.add_node(node("A"));
        let b = graph.add_node(node("B"));
        graph.add_edge(a, b, Edge::new(EdgeKind::References));
        graph.add_edge(a, b, Edge::new(EdgeKind::References));

        let found = graph.related(b, &[EdgeKind::References], Direction::Incoming);
        assert_eq!(found.len(), 1, "two parallel edges must yield one node");
    }

    #[test]
    fn direction_is_respected() {
        let mut graph = ArborGraph::new();
        let a = graph.add_node(node("A"));
        let b = graph.add_node(node("B"));
        graph.add_edge(a, b, Edge::new(EdgeKind::UsesType));

        let incoming = graph.related(b, &[EdgeKind::UsesType], Direction::Incoming);
        assert_eq!(incoming.len(), 1);
        assert_eq!(incoming[0].name, "A");

        assert!(graph
            .related(b, &[EdgeKind::UsesType], Direction::Outgoing)
            .is_empty());
        assert!(graph
            .related(a, &[EdgeKind::UsesType], Direction::Incoming)
            .is_empty());
    }

    #[test]
    fn multiple_kinds_return_the_union() {
        let mut graph = ArborGraph::new();
        let a = graph.add_node(node("A"));
        let b = graph.add_node(node("B"));
        let c = graph.add_node(node("C"));
        graph.add_edge(a, b, Edge::new(EdgeKind::Calls));
        graph.add_edge(c, b, Edge::new(EdgeKind::UsesType));

        let mut names: Vec<&str> = graph
            .related(
                b,
                &[EdgeKind::Calls, EdgeKind::UsesType],
                Direction::Incoming,
            )
            .iter()
            .map(|n| n.name.as_str())
            .collect();
        names.sort();
        assert_eq!(names, vec!["A", "C"]);
    }

    #[test]
    fn relationships_by_kind_groups_and_omits_empty_kinds() {
        let mut graph = ArborGraph::new();
        let a = graph.add_node(node("A"));
        let b = graph.add_node(node("B"));
        let c = graph.add_node(node("C"));
        graph.add_edge(a, b, Edge::new(EdgeKind::Calls));
        graph.add_edge(a, b, Edge::new(EdgeKind::UsesType));
        graph.add_edge(a, c, Edge::new(EdgeKind::UsesType));

        let grouped = graph.relationships_by_kind(a, Direction::Outgoing);

        assert_eq!(grouped.len(), 2, "only kinds with edges are present");
        assert!(!grouped.contains_key(&EdgeKind::References));

        let calls: Vec<&str> = grouped[&EdgeKind::Calls]
            .iter()
            .map(|n| n.name.as_str())
            .collect();
        assert_eq!(calls, vec!["B"]);

        let mut uses_type: Vec<&str> = grouped[&EdgeKind::UsesType]
            .iter()
            .map(|n| n.name.as_str())
            .collect();
        uses_type.sort();
        assert_eq!(uses_type, vec!["B", "C"]);
    }

    #[test]
    fn has_edges_of_kind_is_true_for_present_and_false_for_absent() {
        let mut graph = ArborGraph::new();
        let a = graph.add_node(node("A"));
        let b = graph.add_node(node("B"));
        graph.add_edge(a, b, Edge::new(EdgeKind::UsesType));

        assert!(graph.has_edges_of_kind(&[EdgeKind::UsesType]));
        assert!(!graph.has_edges_of_kind(&[EdgeKind::References]));
    }

    #[test]
    fn related_transitive_respects_max_depth_and_excludes_the_start_node() {
        // A -UsesType-> B -UsesType-> C -UsesType-> D
        let mut graph = ArborGraph::new();
        let a = graph.add_node(node("A"));
        let b = graph.add_node(node("B"));
        let c = graph.add_node(node("C"));
        let d = graph.add_node(node("D"));
        graph.add_edge(a, b, Edge::new(EdgeKind::UsesType));
        graph.add_edge(b, c, Edge::new(EdgeKind::UsesType));
        graph.add_edge(c, d, Edge::new(EdgeKind::UsesType));

        let found = graph.related_transitive(a, &[EdgeKind::UsesType], Direction::Outgoing, 2);
        let names: Vec<&str> = found.iter().map(|(n, _)| n.name.as_str()).collect();

        assert_eq!(names, vec!["B", "C"], "depth 2 stops before D");
        assert!(
            !names.contains(&"A"),
            "the start node must never appear in its own results"
        );
    }
}
