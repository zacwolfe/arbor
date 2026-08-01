//! Reports graph density for a directory. Used to measure edge-recall changes.
//!
//! Usage: cargo run -p arbor-watcher --example graph_stats -- <dir>

use arbor_watcher::{index_directory, IndexOptions};
use std::path::Path;

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| ".".to_string());
    let result = index_directory(Path::new(&dir), IndexOptions::default()).expect("index failed");
    let graph = result.graph;

    let nodes = graph.node_count();
    let edges = graph.edge_count();

    let mut confident = 0usize;
    let mut weak = 0usize;
    for w in graph.edge_weights() {
        if w.is_confident() {
            confident += 1;
        } else {
            weak += 1;
        }
    }

    println!("dir            {dir}");
    println!("files          {}", result.files_indexed);
    println!("nodes          {nodes}");
    println!("edges          {edges}");
    println!(
        "edges/node     {:.3}",
        if nodes > 0 {
            edges as f64 / nodes as f64
        } else {
            0.0
        }
    );
    println!("  confident    {confident}");
    println!("  weak         {weak}");
    println!("orphan nodes   {}", orphans(&graph));
}

fn orphans(graph: &arbor_graph::ArborGraph) -> usize {
    graph
        .node_indexes()
        .filter(|&i| graph.get_callers(i).is_empty() && graph.get_callees(i).is_empty())
        .count()
}
