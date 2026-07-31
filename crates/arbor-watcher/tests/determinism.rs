//! Indexing the same source twice must produce the same graph.
//!
//! Arbor's headline claim is a deterministic walk. That claim used to be false:
//! symbol resolution picked a winner by iterating a `HashMap`, and Rust seeds
//! `RandomState` per process, so the same binary on the same input could build
//! different edges between runs.
//!
//! These tests would not have caught it by running the graph builder twice
//! inside one process — a single process shares one hash seed. They shell out
//! to fresh processes, which is the only way to vary the seed.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use arbor_watcher::{index_directory, IndexOptions};

/// A canonical, order-independent description of a graph.
fn signature(graph: &arbor_graph::ArborGraph) -> String {
    let nodes: BTreeSet<String> = graph
        .node_indexes()
        .filter_map(|i| graph.get(i))
        .map(|n| {
            format!(
                "N {} {} {} {}:{}",
                n.id, n.qualified_name, n.file, n.line_start, n.line_end
            )
        })
        .collect();

    let edges: BTreeSet<String> = graph
        .export_edges()
        .into_iter()
        .map(|e| format!("E {} -> {} [{}]", e.source, e.target, e.kind))
        .collect();

    let mut out = String::new();
    for line in nodes.iter().chain(edges.iter()) {
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn fixture_dir(name: &str) -> std::path::PathBuf {
    // Each test gets its own directory: these run concurrently, and a shared
    // path meant one test could delete the fixture another was mid-way
    // through indexing.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/determinism")
        .join(name)
}

/// Writes a fixture designed to exercise every ambiguous resolution path:
/// colliding bare names across directories, same-name methods on different
/// classes, and unknown-receiver method calls.
fn write_fixture(name: &str) -> std::path::PathBuf {
    let dir = fixture_dir(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("a")).unwrap();
    std::fs::create_dir_all(dir.join("b")).unwrap();

    std::fs::write(
        dir.join("a/service.ts"),
        r#"
export class UserService {
  findOne(id: string) { return validate(id); }
  save(u: unknown) { return this.findOne("x"); }
}
export function validate(v: string) { return v.length > 0; }
"#,
    )
    .unwrap();

    std::fs::write(
        dir.join("b/service.ts"),
        r#"
export class OrderService {
  findOne(id: string) { return validate(id); }
  process(o: unknown) { return this.findOne("y"); }
}
export function validate(v: string) { return !!v; }
"#,
    )
    .unwrap();

    std::fs::write(
        dir.join("main.ts"),
        r#"
import { UserService } from "./a/service";
export function handler(svc: UserService) {
  const r = svc.findOne("1");
  return validate(r);
}
export function validate(x: unknown) { return !!x; }
"#,
    )
    .unwrap();

    dir
}

#[test]
fn same_process_repeat_indexing_is_stable() {
    let dir = write_fixture("same_process");

    let first = index_directory(&dir, IndexOptions::default()).unwrap();
    let baseline = signature(&first.graph);

    for round in 0..5 {
        let again = index_directory(&dir, IndexOptions::default()).unwrap();
        assert_eq!(
            baseline,
            signature(&again.graph),
            "graph changed on round {round}"
        );
    }
}

/// The real test: separate processes, therefore separate hash seeds.
///
/// Runs the `graph_stats`-style signature dump via a helper binary invocation
/// of this same test executable, so no extra build target is needed.
#[test]
fn cross_process_indexing_is_stable() {
    let dir = write_fixture("cross_process");

    // Child mode: print the signature and exit.
    if std::env::var("ARBOR_DETERMINISM_CHILD").is_ok() {
        let result = index_directory(&dir, IndexOptions::default()).unwrap();
        print!("{}", signature(&result.graph));
        return;
    }

    let exe = std::env::current_exe().expect("test executable path");
    let mut signatures = BTreeSet::new();

    for round in 0..8 {
        let output = Command::new(&exe)
            .arg("cross_process_indexing_is_stable")
            .arg("--exact")
            .arg("--nocapture")
            .env("ARBOR_DETERMINISM_CHILD", "1")
            .output()
            .expect("spawn child test process");

        assert!(
            output.status.success(),
            "child round {round} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        // Strip libtest's own framing; keep only our N/E lines.
        let sig: String = stdout
            .lines()
            .filter(|l| l.starts_with("N ") || l.starts_with("E "))
            .map(|l| format!("{l}\n"))
            .collect();

        assert!(!sig.is_empty(), "child produced no signature");
        signatures.insert(sig);
    }

    assert_eq!(
        signatures.len(),
        1,
        "indexing produced {} distinct graphs across processes — resolution is not deterministic",
        signatures.len()
    );
}

#[test]
fn colliding_symbols_are_all_indexed() {
    // Three files define `validate`; the old symbol table kept one and
    // silently orphaned the rest.
    let dir = write_fixture("collisions");
    let result = index_directory(&dir, IndexOptions::default()).unwrap();

    let validates = result
        .graph
        .node_indexes()
        .filter_map(|i| result.graph.get(i))
        .filter(|n| n.name == "validate")
        .count();

    assert_eq!(validates, 3, "every colliding definition must be a node");
}

#[test]
fn edge_confidence_stays_in_range() {
    let dir = write_fixture("confidence");
    let result = index_directory(&dir, IndexOptions::default()).unwrap();

    for edge in result.graph.edge_weights() {
        assert!(
            (0.0..=1.0).contains(&edge.confidence),
            "confidence out of range: {}",
            edge.confidence
        );
    }
}
