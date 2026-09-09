//! Reading the on-disk graph cache, and knowing where it came from.
//!
//! Lives here rather than in `arbor-cli` because more than one front end needs
//! it. The GUI previously re-indexed with Tree-sitter on every launch, so on a
//! project whose graph came from a SCIP index it displayed a materially
//! different graph from the CLI — same repo, half the edges, and methods with
//! hundreds of callers shown as uncalled.

use crate::graph::ArborGraph;
use std::path::{Path, PathBuf};

/// Bincode-serialised graph, including centrality scores.
pub fn graph_binary_path(project_root: &Path) -> PathBuf {
    project_root.join(".arbor").join("graph.bin")
}

/// JSON snapshot of the graph.
pub fn graph_snapshot_path(project_root: &Path) -> PathBuf {
    project_root.join(".arbor").join("graph.json")
}

/// Marker recording that the cached graph was built from a SCIP index.
pub fn scip_provenance_path(project_root: &Path) -> PathBuf {
    project_root.join(".arbor").join("scip.json")
}

/// The SCIP index files the cached graph was built from, if any.
///
/// `Some` means the graph is compiler-resolved and must not be replaced by a
/// Tree-sitter re-index. A malformed marker reads as `None`: it is a hint, and
/// refusing to work because a hint is corrupt would be worse than ignoring it.
pub fn scip_indexes(project_root: &Path) -> Option<Vec<String>> {
    let contents = std::fs::read_to_string(scip_provenance_path(project_root)).ok()?;
    let value: serde_json::Value = serde_json::from_str(&contents).ok()?;

    Some(
        value
            .get("indexes")?
            .as_array()?
            .iter()
            .filter_map(|entry| entry.as_str().map(str::to_string))
            .collect(),
    )
}

/// Whether this project's graph came from a SCIP index.
pub fn is_scip_provenanced(project_root: &Path) -> bool {
    scip_indexes(project_root).is_some()
}

/// Loads the bincode graph, rebuilding the search index that is not serialised.
pub fn load_binary(project_root: &Path) -> Result<ArborGraph, String> {
    let path = graph_binary_path(project_root);
    let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut graph: ArborGraph =
        bincode::deserialize(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    graph.rebuild_search_index();
    Ok(graph)
}

/// Loads the JSON snapshot, rebuilding the search index.
pub fn load_snapshot(project_root: &Path) -> Result<ArborGraph, String> {
    let path = graph_snapshot_path(project_root);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut graph: ArborGraph =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    graph.rebuild_search_index();
    Ok(graph)
}

/// Loads whichever cached graph is available, preferring the binary form.
pub fn load_any(project_root: &Path) -> Option<ArborGraph> {
    load_binary(project_root)
        .or_else(|_| load_snapshot(project_root))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_marker(dir: &Path, body: &str) {
        std::fs::create_dir_all(dir.join(".arbor")).unwrap();
        std::fs::write(scip_provenance_path(dir), body).unwrap();
    }

    #[test]
    fn no_marker_means_not_scip_provenanced() {
        let dir = TempDir::new().unwrap();
        assert!(!is_scip_provenanced(dir.path()));
        assert!(scip_indexes(dir.path()).is_none());
    }

    #[test]
    fn reads_the_index_list_from_the_marker() {
        let dir = TempDir::new().unwrap();
        write_marker(
            dir.path(),
            r#"{"indexes":["a.scip","b.scip"],"merged":false}"#,
        );

        assert!(is_scip_provenanced(dir.path()));
        assert_eq!(
            scip_indexes(dir.path()).unwrap(),
            vec!["a.scip".to_string(), "b.scip".to_string()]
        );
    }

    #[test]
    fn a_corrupt_marker_reads_as_absent() {
        let dir = TempDir::new().unwrap();
        write_marker(dir.path(), "{not json");
        assert!(!is_scip_provenanced(dir.path()));
    }

    #[test]
    fn a_marker_without_indexes_reads_as_absent() {
        let dir = TempDir::new().unwrap();
        write_marker(dir.path(), r#"{"merged":true}"#);
        assert!(!is_scip_provenanced(dir.path()));
    }

    #[test]
    fn load_any_is_none_when_there_is_no_cache() {
        let dir = TempDir::new().unwrap();
        assert!(load_any(dir.path()).is_none());
    }

    #[test]
    fn round_trips_a_graph_through_the_binary_cache() {
        use arbor_core::{CodeNode, NodeKind};

        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".arbor")).unwrap();

        let mut graph = ArborGraph::new();
        graph.add_node(CodeNode::new(
            "pay",
            "Checkout.pay",
            NodeKind::Method,
            "C.java",
        ));
        std::fs::write(
            graph_binary_path(dir.path()),
            bincode::serialize(&graph).unwrap(),
        )
        .unwrap();

        let loaded = load_any(dir.path()).expect("cache must load");
        assert_eq!(loaded.node_count(), 1);
        // The search index is not serialised; loading must rebuild it.
        assert_eq!(loaded.search("pay").len(), 1);
    }
}
