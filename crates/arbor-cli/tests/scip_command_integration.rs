//! End-to-end coverage for `arbor scip`.
//!
//! The fixture is a real SCIP protobuf payload rather than a mocked struct, so
//! these tests exercise the decode path and the CLI exactly as a `scip-java`
//! artifact from CI would.

use scip::types::{
    occurrence, Document, Index, MultiLineRange, Occurrence, Relationship, SingleLineRange,
    SymbolInformation, SymbolRole,
};
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

const GATEWAY_TYPE: &str = "semanticdb maven . . com/example/Gateway#";
const GATEWAY_CHARGE: &str = "semanticdb maven . . com/example/Gateway#charge().";
const STRIPE_CHARGE: &str = "semanticdb maven . . com/example/StripeGateway#charge().";
const CHECKOUT_PAY: &str = "semanticdb maven . . com/example/Checkout#pay().";
const CHECKOUT_TYPE: &str = "semanticdb maven . . com/example/Checkout#";
const CHECKOUT_GATEWAY_FIELD: &str = "semanticdb maven . . com/example/Checkout#gateway.";
const CHECKOUT_TEST_TYPE: &str = "semanticdb maven . . com/example/CheckoutTest#";
const CHECKOUT_TEST_GATEWAY_FIELD: &str = "semanticdb maven . . com/example/CheckoutTest#gateway.";
const CHECKOUT_TEST_CHARGE: &str = "semanticdb maven . . com/example/CheckoutTest#testCharge().";

/// Runs arbor with auto-rebuild disabled.
///
/// Without this, any test whose index looks stale would shell out to whatever
/// `scip-java` happens to be installed on the developer's machine — slow, and
/// dependent on state no test controls. Tests that exercise the rebuild opt in
/// via [`run_arbor_with_path`], which supplies a stub.
fn run_arbor(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_arbor"))
        .args(args)
        .current_dir(dir)
        .env("ARBOR_NO_AUTO_REBUILD", "1")
        .output()
        .expect("failed to run arbor")
}

/// Runs arbor with a PATH that deliberately contains no `scip-java`.
fn run_arbor_without_scip_java(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_arbor"))
        .args(args)
        .current_dir(dir)
        .env("PATH", "/nonexistent-for-tests")
        .output()
        .expect("failed to run arbor")
}

fn run_arbor_stdout(dir: &Path, args: &[&str]) -> String {
    let output = run_arbor(dir, args);
    assert!(
        output.status.success(),
        "arbor {:?} failed:\nstdout: {}\nstderr: {}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn single_line(line: i32) -> SingleLineRange {
    let mut range = SingleLineRange::new();
    range.line = line;
    range.start_character = 4;
    range.end_character = 24;
    range
}

fn multi_line(start: i32, end: i32) -> MultiLineRange {
    let mut range = MultiLineRange::new();
    range.start_line = start;
    range.start_character = 4;
    range.end_line = end;
    range.end_character = 5;
    range
}

/// Uses the typed range encoding — the only one current `scip-java` emits.
/// Lines are 0-indexed, as SCIP requires.
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

fn implements(symbol: &str, supertype: &str) -> SymbolInformation {
    let mut relationship = Relationship::new();
    relationship.symbol = supertype.to_string();
    relationship.is_implementation = true;

    let mut info = SymbolInformation::new();
    info.symbol = symbol.to_string();
    info.relationships = vec![relationship];
    info
}

fn document(
    relative_path: &str,
    occurrences: Vec<Occurrence>,
    symbols: Vec<SymbolInformation>,
) -> Document {
    let mut document = Document::new();
    document.relative_path = relative_path.to_string();
    document.language = "Java".to_string();
    document.occurrences = occurrences;
    document.symbols = symbols;
    document
}

/// A three-file Java project: an interface, one implementation, one caller
/// that only ever names the interface.
fn setup_java_project() -> TempDir {
    let temp = TempDir::new().expect("create temp dir");
    let dir = temp.path();

    // `pom.xml` doubles as the workspace-root marker Arbor looks for.
    fs::write(dir.join("pom.xml"), "<project/>\n").expect("write pom.xml");

    let sources = dir.join("src/main/java/com/example");
    fs::create_dir_all(&sources).expect("create source dirs");
    fs::write(
        sources.join("Gateway.java"),
        "package com.example;\n\npublic interface Gateway {\n    void charge();\n}\n",
    )
    .expect("write Gateway.java");
    fs::write(
        sources.join("StripeGateway.java"),
        "package com.example;\n\npublic class StripeGateway implements Gateway {\n    public void charge() {}\n}\n",
    )
    .expect("write StripeGateway.java");
    fs::write(
        sources.join("Checkout.java"),
        "package com.example;\n\npublic class Checkout {\n    private Gateway gateway;\n    public void pay() {\n        gateway.charge();\n    }\n}\n",
    )
    .expect("write Checkout.java");

    let mut index = Index::new();
    index.documents = vec![
        document(
            "src/main/java/com/example/Gateway.java",
            vec![
                definition(GATEWAY_TYPE, 2, 4),
                definition(GATEWAY_CHARGE, 3, 3),
            ],
            vec![],
        ),
        document(
            "src/main/java/com/example/StripeGateway.java",
            vec![definition(STRIPE_CHARGE, 3, 3)],
            vec![implements(STRIPE_CHARGE, GATEWAY_CHARGE)],
        ),
        document(
            "src/main/java/com/example/Checkout.java",
            vec![
                definition(CHECKOUT_PAY, 4, 6),
                // `gateway.charge()` — the call Tree-sitter has to drop because
                // it cannot type the receiver.
                reference(GATEWAY_CHARGE, 5),
            ],
            vec![],
        ),
    ];

    scip::write_message_to_file(dir.join("index.scip"), index).expect("write index.scip");

    temp
}

/// The same three-file Java project as [`setup_java_project`], with the
/// `references`/`uses_type` edges that command needs and the plain
/// `Calls`/`Implements` fixture does not exercise.
///
/// `Checkout.java`'s existing text already has the shapes these edges need:
/// `private Gateway gateway;` (line 3, 0-indexed) is a field typed by an
/// interface, and `gateway.charge()` (line 5) is a field access next to the
/// method call the original fixture already covers. Adding the Checkout type
/// and field as definitions, plus occurrences naming what they refer to, is
/// enough for `reference_edge_kind` (`arbor-scip/src/ingest.rs`) to type them:
/// a reference whose target is a class/interface becomes `UsesType`, and a
/// reference whose target is a plain field becomes `References`.
///
/// Also adds `src/test/java/com/example/CheckoutTest.java`, a test-file
/// source with its own `gateway` field typed as `Gateway` (a second incoming
/// `uses_type` edge on `Gateway`, distinct from `Checkout.gateway`) and its
/// own `testCharge()` method referencing that field (a `references` edge that
/// targets a symbol nothing else in this fixture touches, so it cannot change
/// the `Checkout.gateway` incoming-reference count another test asserts on).
/// This is what exercises `--exclude-test` for `uses-type`/`references`.
fn setup_java_project_with_relationships() -> TempDir {
    let temp = TempDir::new().expect("create temp dir");
    let dir = temp.path();

    fs::write(dir.join("pom.xml"), "<project/>\n").expect("write pom.xml");

    let sources = dir.join("src/main/java/com/example");
    fs::create_dir_all(&sources).expect("create source dirs");
    fs::write(
        sources.join("Gateway.java"),
        "package com.example;\n\npublic interface Gateway {\n    void charge();\n}\n",
    )
    .expect("write Gateway.java");
    fs::write(
        sources.join("StripeGateway.java"),
        "package com.example;\n\npublic class StripeGateway implements Gateway {\n    public void charge() {}\n}\n",
    )
    .expect("write StripeGateway.java");
    fs::write(
        sources.join("Checkout.java"),
        "package com.example;\n\npublic class Checkout {\n    private Gateway gateway;\n    public void pay() {\n        gateway.charge();\n    }\n}\n",
    )
    .expect("write Checkout.java");

    let test_sources = dir.join("src/test/java/com/example");
    fs::create_dir_all(&test_sources).expect("create test source dirs");
    fs::write(
        test_sources.join("CheckoutTest.java"),
        "package com.example;\n\npublic class CheckoutTest {\n    private Gateway gateway;\n    public void testCharge() {\n        gateway.charge();\n    }\n}\n",
    )
    .expect("write CheckoutTest.java");

    let mut index = Index::new();
    index.documents = vec![
        document(
            "src/main/java/com/example/Gateway.java",
            vec![
                definition(GATEWAY_TYPE, 2, 4),
                definition(GATEWAY_CHARGE, 3, 3),
            ],
            vec![],
        ),
        document(
            "src/main/java/com/example/StripeGateway.java",
            vec![definition(STRIPE_CHARGE, 3, 3)],
            vec![implements(STRIPE_CHARGE, GATEWAY_CHARGE)],
        ),
        document(
            "src/main/java/com/example/Checkout.java",
            vec![
                definition(CHECKOUT_TYPE, 2, 7),
                definition(CHECKOUT_GATEWAY_FIELD, 3, 3),
                definition(CHECKOUT_PAY, 4, 6),
                // `private Gateway gateway;` — the field's declared type. The
                // innermost enclosing definition at line 3 is the field
                // itself, so this becomes `Checkout.gateway --uses_type--> Gateway`.
                reference(GATEWAY_TYPE, 3),
                // `gateway.charge()` — the call Tree-sitter has to drop
                // because it cannot type the receiver (unchanged from the
                // plain fixture).
                reference(GATEWAY_CHARGE, 5),
                // The same line's `gateway` field access. The enclosing
                // definition at line 5 is `pay()`, so this becomes
                // `Checkout.pay --references--> Checkout.gateway`.
                reference(CHECKOUT_GATEWAY_FIELD, 5),
            ],
            vec![],
        ),
        document(
            "src/test/java/com/example/CheckoutTest.java",
            vec![
                definition(CHECKOUT_TEST_TYPE, 2, 6),
                definition(CHECKOUT_TEST_GATEWAY_FIELD, 3, 3),
                definition(CHECKOUT_TEST_CHARGE, 4, 6),
                // A second field typed as `Gateway`, so `uses-type Gateway`
                // has a test-file result to filter with `--exclude-test`.
                reference(GATEWAY_TYPE, 3),
                // References its own field, not `Checkout.gateway` — this
                // must not perturb the incoming-reference count another test
                // asserts on for `Checkout.gateway`.
                reference(CHECKOUT_TEST_GATEWAY_FIELD, 5),
            ],
            vec![],
        ),
    ];

    scip::write_message_to_file(dir.join("index.scip"), index).expect("write index.scip");

    temp
}

#[test]
fn scip_ingest_builds_a_graph_and_reports_counts() {
    let temp = setup_java_project();
    let dir = temp.path();

    let stdout = run_arbor_stdout(dir, &["scip", "index.scip", "--root", "."]);

    assert!(stdout.contains("3 documents"), "got: {stdout}");
    assert!(stdout.contains("4 definitions"), "got: {stdout}");
    assert!(dir.join(".arbor/graph.bin").exists());
}

#[test]
fn scip_json_output_is_machine_readable() {
    let temp = setup_java_project();
    let dir = temp.path();

    let stdout = run_arbor_stdout(dir, &["scip", "index.scip", "--root", ".", "--json"]);
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");

    assert_eq!(value["definitions"], 4);
    assert_eq!(value["documents"], 3);
    assert_eq!(value["referencesResolved"], 1);
    assert_eq!(value["implementsEdges"], 1);
    assert_eq!(value["dispatchEdges"], 1);
    assert_eq!(value["exactEdgesDropped"], 0);
}

#[test]
fn dotted_method_call_becomes_a_real_edge() {
    let temp = setup_java_project();
    let dir = temp.path();

    run_arbor_stdout(dir, &["scip", "index.scip", "--root", "."]);

    // This is the whole point: `gateway.charge()` is invisible to Tree-sitter.
    let stdout = run_arbor_stdout(dir, &["callees", "pay", "."]);
    assert!(
        stdout.contains("com.example.Gateway.charge"),
        "expected the resolved interface call, got: {stdout}"
    );
}

#[test]
fn virtual_dispatch_reaches_the_implementation() {
    let temp = setup_java_project();
    let dir = temp.path();

    run_arbor_stdout(dir, &["scip", "index.scip", "--root", "."]);

    let stdout = run_arbor_stdout(dir, &["callees", "pay", "."]);
    assert!(
        stdout.contains("com.example.StripeGateway.charge"),
        "expected dispatch expansion to reach the implementation, got: {stdout}"
    );
}

#[test]
fn dispatch_expansion_can_be_disabled() {
    let temp = setup_java_project();
    let dir = temp.path();

    run_arbor_stdout(dir, &["scip", "index.scip", "--root", ".", "--no-dispatch"]);

    let stdout = run_arbor_stdout(dir, &["callees", "pay", "."]);
    assert!(
        stdout.contains("com.example.Gateway.charge"),
        "the declared call must survive, got: {stdout}"
    );
    assert!(
        !stdout.contains("com.example.StripeGateway.charge"),
        "expected no synthesised dispatch edge, got: {stdout}"
    );
}

#[test]
fn merge_keeps_files_the_scip_index_does_not_cover() {
    let temp = setup_java_project();
    let dir = temp.path();

    // A non-JVM source the SCIP index knows nothing about.
    fs::write(
        dir.join("tooling.py"),
        "def reconcile_ledger():\n    return 1\n",
    )
    .expect("write tooling.py");

    run_arbor_stdout(dir, &["scip", "index.scip", "--root", ".", "--merge"]);

    let stdout = run_arbor_stdout(dir, &["query", "reconcile_ledger", "."]);
    assert!(
        stdout.contains("reconcile_ledger"),
        "expected the Python symbol to survive the merge, got: {stdout}"
    );

    // ...and the SCIP-resolved call must still be there.
    let callees = run_arbor_stdout(dir, &["callees", "pay", "."]);
    assert!(
        callees.contains("com.example.StripeGateway.charge"),
        "expected SCIP edges to survive the merge, got: {callees}"
    );
}

#[test]
fn a_stale_cache_does_not_silently_discard_the_scip_graph() {
    let temp = setup_java_project();
    let dir = temp.path();

    run_arbor_stdout(dir, &["scip", "index.scip", "--root", "."]);

    // Make a source look newer than the cache. Staleness is compared in whole
    // seconds, so an ordinary write lands in the same second as the cache and
    // would not register — set the mtime forward explicitly.
    let checkout = dir.join("src/main/java/com/example/Checkout.java");
    let contents = fs::read_to_string(&checkout).expect("read Checkout.java");
    fs::write(&checkout, format!("{contents}// touched\n")).expect("touch Checkout.java");

    let handle = fs::OpenOptions::new()
        .write(true)
        .open(&checkout)
        .expect("open Checkout.java");
    handle
        .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(10))
        .expect("set mtime");
    drop(handle);

    let output = run_arbor(dir, &["callees", "pay", "."]);
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        stdout.contains("com.example.StripeGateway.charge"),
        "the SCIP graph must be served, not rebuilt: {stdout}"
    );
    assert!(
        stderr.contains("arbor scip"),
        "expected a warning naming the refresh command, got: {stderr}"
    );
}

#[test]
fn a_missing_index_fails_before_doing_any_work() {
    let temp = setup_java_project();
    let dir = temp.path();

    let output = run_arbor(dir, &["scip", "nope.scip", "--root", "."]);
    assert!(!output.status.success());

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("nope.scip"),
        "expected the bad path named in the error, got: {stderr}"
    );
}

#[test]
fn a_non_scip_file_reports_a_decode_error() {
    let temp = setup_java_project();
    let dir = temp.path();

    // A JSON index that was never converted to protobuf — a real mistake, and
    // one that should not read as an Arbor bug.
    fs::write(dir.join("bogus.scip"), r#"{"documents": []}"#).expect("write bogus.scip");

    let output = run_arbor(dir, &["scip", "bogus.scip", "--root", "."]);
    assert!(!output.status.success());

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("not a valid SCIP index"), "got: {stderr}");
}

#[test]
fn a_read_command_never_rebuilds_over_a_scip_graph() {
    let temp = setup_java_project();
    let dir = temp.path();

    run_arbor_stdout(dir, &["scip", "index.scip", "--root", "."]);

    // Force the cache-load path to fail, which is what previously fell through
    // to a Tree-sitter rebuild *and persisted it* — destroying the SCIP graph
    // as a side effect of a plain read.
    fs::remove_file(dir.join(".arbor/graph.bin")).expect("remove graph.bin");
    fs::remove_file(dir.join(".arbor/graph.json")).expect("remove graph.json");

    let output = run_arbor(dir, &["callers", "pay", "."]);
    assert!(
        !output.status.success(),
        "must refuse, not silently rebuild"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("built from a SCIP index"),
        "expected the provenance refusal, got: {stderr}"
    );
    assert!(
        !dir.join(".arbor/graph.bin").exists(),
        "a Tree-sitter graph must not have been written"
    );
}

#[test]
fn index_refuses_on_a_scip_project_without_force() {
    let temp = setup_java_project();
    let dir = temp.path();

    run_arbor_stdout(dir, &["scip", "index.scip", "--root", "."]);

    let output = run_arbor(dir, &["index", "."]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--force"));
    assert!(
        dir.join(".arbor/scip.json").exists(),
        "a refused index must leave the marker in place"
    );
}

#[test]
fn changed_only_refuses_too() {
    // The incremental path is the worst case, not the mildest: a Tree-sitter
    // patch merged into a SCIP graph leaves both naming schemes in one graph.
    let temp = setup_java_project();
    let dir = temp.path();

    run_arbor_stdout(dir, &["scip", "index.scip", "--root", "."]);

    let output = run_arbor(dir, &["index", ".", "--changed-only"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("built from a SCIP index"));
}

#[test]
fn index_force_proceeds_and_clears_the_marker() {
    let temp = setup_java_project();
    let dir = temp.path();

    run_arbor_stdout(dir, &["scip", "index.scip", "--root", "."]);
    run_arbor_stdout(dir, &["index", ".", "--force"]);

    assert!(
        !dir.join(".arbor/scip.json").exists(),
        "--force is the deliberate downgrade, so the marker must go"
    );
    // And normal indexing works again afterwards.
    run_arbor_stdout(dir, &["index", "."]);
}

#[test]
fn status_reports_the_scip_graph_not_a_fresh_tree_sitter_one() {
    let temp = setup_java_project();
    let dir = temp.path();

    run_arbor_stdout(dir, &["scip", "index.scip", "--root", "."]);

    let stdout = run_arbor_stdout(dir, &["status", "."]);
    assert!(
        stdout.contains("SCIP index"),
        "status must name its source, got: {stdout}"
    );
}

#[test]
fn export_emits_the_scip_graph() {
    let temp = setup_java_project();
    let dir = temp.path();

    run_arbor_stdout(dir, &["scip", "index.scip", "--root", "."]);
    run_arbor_stdout(dir, &["export", "--output", "g.json", "."]);

    let text = fs::read_to_string(dir.join("g.json")).expect("read export");
    let value: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
    // 4 SCIP definitions, not the Tree-sitter node count for the same tree.
    assert_eq!(value["stats"]["nodeCount"], 4);
}

/// Puts a fake `scip-java` on PATH so the pipeline can be exercised without a
/// real Gradle build. It reproduces the configuration-cache failure on the
/// bare invocation and only succeeds once the task list is passed back.
fn stub_scip_java(dir: &Path, succeed_on_retry: bool) -> std::path::PathBuf {
    let bin = dir.join("stub-bin");
    fs::create_dir_all(&bin).expect("create stub bin");
    let script = bin.join("scip-java");

    let payload = match succeed_on_retry {
        true => "cp fixture.scip index.scip\necho BUILD SUCCESSFUL\n",
        false => "echo 'error: cannot find symbol Foo' >&2\nexit 1\n",
    };

    fs::write(
        &script,
        format!(
            "#!/usr/bin/env bash\n\
             if [[ \"$*\" == \"--version\" || \"$*\" == \"--help\" ]]; then\n\
               echo 'scip-java stub'; exit 0\n\
             fi\n\
             if [[ \"$*\" == \"index\" ]]; then\n\
               echo '$ ./gradlew --no-daemon --init-script /tmp/x/init-script.gradle clean scipPrintDependencies scipCompileAll'\n\
               echo 'Configuration cache problems found in this build.'\n\
               exit 1\n\
             fi\n\
             echo \"RETRY: $*\"\n{payload}"
        ),
    )
    .expect("write stub");

    let mut perms = fs::metadata(&script).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
    fs::set_permissions(&script, perms).unwrap();

    bin
}

fn run_arbor_with_path(dir: &Path, bin: &Path, args: &[&str]) -> Output {
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    Command::new(env!("CARGO_BIN_EXE_arbor"))
        .args(args)
        .current_dir(dir)
        .env("PATH", path)
        .output()
        .expect("failed to run arbor")
}

#[test]
fn background_rebuild_retries_the_config_cache_and_swaps_the_graph() {
    let temp = setup_java_project();
    let dir = temp.path();
    fs::copy(dir.join("index.scip"), dir.join("fixture.scip")).expect("stash fixture");
    fs::remove_file(dir.join("index.scip")).expect("remove index so the build must produce it");

    let bin = stub_scip_java(dir, true);
    let output = run_arbor_with_path(dir, &bin, &["scip", "--background", "--root", "."]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Rebuild started"));

    // Poll until terminal rather than sleeping a fixed amount.
    let mut status = String::new();
    for _ in 0..100 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        status = String::from_utf8_lossy(
            &run_arbor(dir, &["scip", "--task-status", "--root", ".", "--json"]).stdout,
        )
        .to_string();
        if status.contains("completed") || status.contains("failed") {
            break;
        }
    }

    assert!(
        status.contains("\"completed\""),
        "task never completed: {status}"
    );
    assert!(
        dir.join(".arbor/graph.bin").exists(),
        "graph must be swapped in"
    );
    assert!(
        dir.join(".arbor/scip.json").exists(),
        "provenance must be written"
    );
}

#[test]
fn a_failed_background_build_leaves_the_graph_untouched() {
    let temp = setup_java_project();
    let dir = temp.path();

    // Establish a good graph first, so there is something to preserve.
    run_arbor_stdout(dir, &["scip", "index.scip", "--root", "."]);
    let before = fs::read(dir.join(".arbor/graph.bin")).expect("read graph");

    let bin = stub_scip_java(dir, false);
    run_arbor_with_path(dir, &bin, &["scip", "--background", "--root", "."]);

    let mut status = String::new();
    for _ in 0..100 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        status = String::from_utf8_lossy(
            &run_arbor(dir, &["scip", "--task-status", "--root", ".", "--json"]).stdout,
        )
        .to_string();
        if status.contains("completed") || status.contains("failed") {
            break;
        }
    }

    assert!(
        status.contains("\"failed\""),
        "expected failure, got: {status}"
    );
    let after = fs::read(dir.join(".arbor/graph.bin")).expect("graph must still exist");
    assert_eq!(before, after, "a failed rebuild must not modify the graph");
}

#[test]
fn task_status_without_a_task_is_not_an_error() {
    let temp = setup_java_project();
    let dir = temp.path();
    run_arbor_stdout(dir, &["scip", "index.scip", "--root", "."]);

    let stdout = run_arbor_stdout(dir, &["scip", "--task-status", "--root", "."]);
    assert!(stdout.contains("No detached SCIP rebuild"), "got: {stdout}");
}

#[test]
fn scip_with_no_index_and_no_flag_explains_itself() {
    let temp = setup_java_project();
    let dir = temp.path();

    let output = run_arbor(dir, &["scip", "--root", "."]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--background"), "got: {stderr}");
}

#[test]
fn a_killed_worker_does_not_lock_out_later_rebuilds() {
    let temp = setup_java_project();
    let dir = temp.path();

    // Hand-write a record naming a PID that cannot be running: an interrupted
    // worker previously blocked every rebuild until an arbitrary timeout.
    fs::create_dir_all(dir.join(".arbor")).unwrap();
    fs::write(
        dir.join(".arbor/scip-task.json"),
        r#"{"id":"scip-old","status":"running","progress":10,"message":"Running scip-java",
            "pid":2,"log":"/tmp/old.log","started_at":1,"updated_at":1,
            "error":null,"node_count":null,"edge_count":null}"#,
    )
    .unwrap();

    fs::copy(dir.join("index.scip"), dir.join("fixture.scip")).unwrap();
    let bin = stub_scip_java(dir, true);
    let output = run_arbor_with_path(dir, &bin, &["scip", "--background", "--root", "."]);

    assert!(
        output.status.success(),
        "a dead worker must not block a new rebuild: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("no longer running"),
        "expected the interrupted-run notice, got: {stdout}"
    );
}

#[test]
fn a_live_worker_still_blocks_a_second_rebuild() {
    let temp = setup_java_project();
    let dir = temp.path();

    // This test process is alive, so a record naming it must be treated as a
    // running rebuild — the guard has to stay effective in that direction.
    fs::create_dir_all(dir.join(".arbor")).unwrap();
    fs::write(
        dir.join(".arbor/scip-task.json"),
        format!(
            r#"{{"id":"scip-live","status":"running","progress":10,"message":"Running scip-java",
                "pid":{},"log":"/tmp/live.log","started_at":1,"updated_at":1,
                "error":null,"node_count":null,"edge_count":null}}"#,
            std::process::id()
        ),
    )
    .unwrap();

    let bin = stub_scip_java(dir, true);
    let output = run_arbor_with_path(dir, &bin, &["scip", "--background", "--root", "."]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("already running"), "got: {stderr}");
    assert!(
        stderr.contains("kill"),
        "should say how to stop it: {stderr}"
    );
}

/// With scip-java absent there is nothing to rebuild with, so the command must
/// still answer from the cache and say how to fix the setup.
#[test]
fn a_dirty_index_without_scip_java_serves_the_cache_and_explains() {
    let temp = setup_java_project();
    let dir = temp.path();

    run_arbor_stdout(dir, &["scip", "index.scip", "--root", "."]);

    let checkout = dir.join("src/main/java/com/example/Checkout.java");
    let contents = fs::read_to_string(&checkout).unwrap();
    fs::write(&checkout, format!("{contents}// touched\n")).unwrap();
    let handle = fs::OpenOptions::new().write(true).open(&checkout).unwrap();
    handle
        .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(10))
        .unwrap();
    drop(handle);

    let output = run_arbor_without_scip_java(dir, &["callers", "pay", "."]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Rebuilding needs the compiler, so the advice must be --background rather
    // than re-ingesting an index that is itself stale.
    assert!(
        stderr.contains("no matching SCIP indexer is installed"),
        "should say the prerequisite is missing, got: {stderr}"
    );
    assert!(
        stderr.contains("scip-java"),
        "should name which indexer this project needs, got: {stderr}"
    );
    assert!(
        stderr.contains("coursier bootstrap"),
        "should give the install command, not just the tool name, got: {stderr}"
    );
    assert!(
        stderr.contains("arbor scip --background"),
        "must recommend --background, got: {stderr}"
    );
    assert!(output.status.success(), "must still answer from the cache");
}

/// The question that motivated the command: which classes implement this
/// interface. Only a compiler index can answer it.
#[test]
fn implementors_finds_the_implementation() {
    let temp = setup_java_project();
    let dir = temp.path();

    run_arbor_stdout(dir, &["scip", "index.scip", "--root", "."]);

    let stdout = run_arbor_stdout(dir, &["implementors", "Gateway.charge", "."]);
    assert!(
        stdout.contains("StripeGateway"),
        "the implementing method must be listed: {stdout}"
    );
}

/// `subclasses` is the word half of users reach for.
#[test]
fn subclasses_is_an_alias() {
    let temp = setup_java_project();
    let dir = temp.path();

    run_arbor_stdout(dir, &["scip", "index.scip", "--root", "."]);
    let stdout = run_arbor_stdout(dir, &["subclasses", "Gateway.charge", "."]);
    assert!(stdout.contains("StripeGateway"), "got: {stdout}");
}

/// Graceful degradation, the hard requirement: a Tree-sitter graph must say it
/// cannot answer, not that there is nothing to find. Reporting "none" here is
/// how someone deletes an interface four classes implement.
#[test]
fn implementors_on_a_tree_sitter_graph_says_it_cannot_answer() {
    let temp = setup_java_project();
    let dir = temp.path();

    // A plain Tree-sitter index — no SCIP anywhere.
    run_arbor_stdout(dir, &["index", "."]);

    let output = run_arbor(dir, &["implementors", "Gateway", "."]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "a missing capability is not a user error: {stdout}"
    );
    assert!(
        stdout.contains("no type hierarchy"),
        "must say the graph cannot answer: {stdout}"
    );
    assert!(
        stdout.contains("Tree-sitter"),
        "must name the producer it is talking about: {stdout}"
    );
    assert!(
        stdout.contains("arbor scip"),
        "must name the way to get an answer: {stdout}"
    );
    assert!(
        !stdout.contains("Nothing implements"),
        "must not report absence as fact: {stdout}"
    );
}

/// A SCIP graph whose indexer *does* carry a hierarchy, where this symbol simply
/// has no implementors, is a different message — and this one may state it.
#[test]
fn implementors_on_a_scip_graph_may_report_a_genuine_absence() {
    let temp = setup_java_project();
    let dir = temp.path();

    run_arbor_stdout(dir, &["scip", "index.scip", "--root", "."]);

    // Checkout.pay implements nothing, on a graph that does carry Implements edges.
    let stdout = run_arbor_stdout(dir, &["implementors", "Checkout.pay", "."]);
    assert!(
        stdout.contains("Nothing implements or extends"),
        "a hierarchy-carrying graph may state the absence: {stdout}"
    );
    assert!(
        stdout.contains("accounted for"),
        "and must say how much that is worth: {stdout}"
    );
}

/// The JSON form has to carry the same distinction, or a tool consuming it
/// re-invents the mistake the human output avoids.
#[test]
fn implementors_json_reports_whether_the_answer_is_knowable() {
    let temp = setup_java_project();
    let dir = temp.path();

    run_arbor_stdout(dir, &["index", "."]);
    let stdout = run_arbor_stdout(dir, &["implementors", "Gateway", ".", "--json"]);
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");

    assert_eq!(value["hierarchyAvailable"], false);
    assert_eq!(value["provenance"], "tree-sitter");
    assert_eq!(value["implementors"].as_array().unwrap().len(), 0);
}

/// "No callers" must never be reported as safety. It used to read "Safe to
/// change, but verify external usage", which is a safety claim derived from
/// absence of evidence.
#[test]
fn an_isolated_node_is_not_declared_safe_to_change() {
    let temp = setup_java_project();
    let dir = temp.path();

    run_arbor_stdout(dir, &["scip", "index.scip", "--root", "."]);

    // The Gateway *type* is defined and never referenced, so it is isolated.
    let stdout = run_arbor_stdout(dir, &["refactor", "Gateway", "."]);

    assert!(
        !stdout.contains("Safe to change"),
        "absence of callers is not evidence of safety:\n{stdout}"
    );
    assert!(
        stdout.contains("not the same as safe to change"),
        "should say what the absence does and does not mean:\n{stdout}"
    );
}

/// How much "no callers" is worth depends on what built the graph, so the
/// wording has to differ: a compiler index resolved every call it could see,
/// Tree-sitter cannot resolve a call on a typed receiver at all.
#[test]
fn the_isolated_verdict_states_which_producer_it_trusts() {
    let temp = setup_java_project();
    let dir = temp.path();

    run_arbor_stdout(dir, &["scip", "index.scip", "--root", "."]);
    let from_scip = run_arbor_stdout(dir, &["refactor", "Gateway", "."]);
    assert!(
        from_scip.contains("came from a compiler index"),
        "a SCIP graph should say in-repo callers are accounted for:\n{from_scip}"
    );

    // Downgrade the same project to Tree-sitter and ask again.
    run_arbor_stdout(dir, &["index", ".", "--force"]);
    let from_tree_sitter = run_arbor_stdout(dir, &["refactor", "Gateway", "."]);
    assert!(
        from_tree_sitter.contains("came from Tree-sitter"),
        "a Tree-sitter graph should admit a caller may be unresolved:\n{from_tree_sitter}"
    );
}

/// A read command that triggers a rebuild must not print the machine-readable
/// stats block into the middle of its own output.
#[test]
fn an_inline_rebuild_does_not_dump_json_into_a_human_command() {
    let temp = setup_java_project();
    let dir = temp.path();

    run_arbor_stdout(dir, &["scip", "index.scip", "--root", "."]);

    // Make the index look stale so the next read rebuilds.
    let checkout = dir.join("src/main/java/com/example/Checkout.java");
    let contents = fs::read_to_string(&checkout).unwrap();
    fs::write(&checkout, format!("{contents}// touched\n")).unwrap();
    let handle = fs::OpenOptions::new().write(true).open(&checkout).unwrap();
    handle
        .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(10))
        .unwrap();
    drop(handle);

    // A stub indexer that reproduces the index, so the rebuild succeeds.
    fs::copy(dir.join("index.scip"), dir.join("fixture.scip")).unwrap();
    let bin = stub_scip_java(dir, true);
    let output = run_arbor_with_path(dir, &bin, &["callers", "pay", "."]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        !stdout.contains("\"definitions\""),
        "the rebuild's JSON stats block leaked into human output:\n{stdout}"
    );
    assert!(
        !stdout.contains("referencesResolved"),
        "the rebuild's JSON stats block leaked into human output:\n{stdout}"
    );
    assert!(
        stdout.contains("Callers of") || stdout.contains("No callers"),
        "the command's own answer must still be printed:\n{stdout}"
    );
}

/// `--background` must refuse before detaching. Launching a worker that cannot
/// possibly run anything sends the user to a log to find out nothing happened.
#[test]
fn background_refuses_up_front_when_no_indexer_is_installed() {
    let temp = setup_java_project();
    let dir = temp.path();

    let output = run_arbor_without_scip_java(dir, &["scip", "--background", "."]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !output.status.success(),
        "must not detach a worker that cannot run: {stderr}"
    );
    assert!(
        stderr.contains("No matching SCIP indexer is installed"),
        "should say what is missing, got: {stderr}"
    );
    assert!(
        stderr.contains("scip-java"),
        "should name the indexer this project needs, got: {stderr}"
    );
    assert!(
        !dir.join(".arbor/scip-task.json").exists(),
        "no task record should be written for a rebuild that never started"
    );
}

/// The Python case that exposed this: a project whose indexer is installed must
/// not be blocked by `scip-java` being absent.
#[test]
fn background_does_not_require_scip_java_for_a_non_jvm_project() {
    let temp = TempDir::new().expect("create temp dir");
    let dir = temp.path();
    fs::write(
        dir.join("pyproject.toml"),
        "[project]\nname = \"scratch\"\n",
    )
    .unwrap();
    fs::create_dir_all(dir.join(".git")).unwrap();

    let output = run_arbor_without_scip_java(dir, &["scip", "--background", "."]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // scip-python is not installed in the test environment either, so this must
    // fail — but naming scip-python, not scip-java.
    assert!(
        stderr.contains("scip-python"),
        "a Python project's missing indexer is scip-python, got: {stderr}"
    );
    assert!(
        !stderr.contains("scip-java"),
        "must not mention the JVM indexer on a Python project, got: {stderr}"
    );
}

/// A project no indexer recognises is a different problem from a missing
/// indexer, and saying "install scip-java" there would send the user nowhere.
#[test]
fn a_dirty_index_with_no_applicable_indexer_says_so() {
    let temp = setup_java_project();
    let dir = temp.path();

    run_arbor_stdout(dir, &["scip", "index.scip", "--root", "."]);

    // Drop the build file so no indexer's markers match. `.git` keeps this a
    // workspace root, which `pom.xml` was doubling as.
    fs::create_dir_all(dir.join(".git")).unwrap();
    for marker in ["build.gradle", "build.gradle.kts", "pom.xml"] {
        let _ = fs::remove_file(dir.join(marker));
    }

    let checkout = dir.join("src/main/java/com/example/Checkout.java");
    let contents = fs::read_to_string(&checkout).unwrap();
    fs::write(&checkout, format!("{contents}// touched\n")).unwrap();
    let handle = fs::OpenOptions::new().write(true).open(&checkout).unwrap();
    handle
        .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(10))
        .unwrap();
    drop(handle);

    let output = run_arbor_without_scip_java(dir, &["callers", "pay", "."]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        stderr.contains("no SCIP indexer matches this project"),
        "should distinguish 'nothing applies' from 'not installed', got: {stderr}"
    );
    assert!(
        stderr.contains("arbor scip <index.scip>"),
        "the only route left is ingesting an index built elsewhere, got: {stderr}"
    );
    assert!(output.status.success(), "must still answer from the cache");
}

/// Marks the SCIP index as ancient so any source file counts as newer.
fn make_index_stale(dir: &Path) {
    let index = dir.join("index.scip");
    let handle = fs::OpenOptions::new().write(true).open(&index).unwrap();
    handle
        .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1))
        .unwrap();
}

#[test]
fn a_dirty_source_triggers_a_blocking_rebuild_on_the_next_command() {
    let temp = setup_java_project();
    let dir = temp.path();

    run_arbor_stdout(dir, &["scip", "index.scip", "--root", "."]);
    fs::copy(dir.join("index.scip"), dir.join("fixture.scip")).unwrap();
    make_index_stale(dir);

    let bin = stub_scip_java(dir, true);
    let output = run_arbor_with_path(dir, &bin, &["status", "."]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        stderr.contains("rebuilding now"),
        "a stale index must trigger a rebuild, got: {stderr}"
    );
    assert!(
        stderr.contains("Graph refreshed"),
        "the rebuild must complete, got: {stderr}"
    );
}

#[test]
fn a_fresh_index_triggers_nothing() {
    let temp = setup_java_project();
    let dir = temp.path();
    run_arbor_stdout(dir, &["scip", "index.scip", "--root", "."]);

    let bin = stub_scip_java(dir, true);
    let output = run_arbor_with_path(dir, &bin, &["status", "."]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !stderr.contains("rebuilding"),
        "an up-to-date index must not rebuild, got: {stderr}"
    );
}

#[test]
fn the_rebuild_does_not_loop_once_it_succeeds() {
    let temp = setup_java_project();
    let dir = temp.path();

    run_arbor_stdout(dir, &["scip", "index.scip", "--root", "."]);
    fs::copy(dir.join("index.scip"), dir.join("fixture.scip")).unwrap();
    make_index_stale(dir);

    let bin = stub_scip_java(dir, true);
    run_arbor_with_path(dir, &bin, &["status", "."]);

    // The rebuild refreshed index.scip, so the second call has nothing to do.
    let second = run_arbor_with_path(dir, &bin, &["status", "."]);
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(
        !stderr.contains("rebuilding now"),
        "must not rebuild again after a successful refresh: {stderr}"
    );
}

#[test]
fn a_failed_rebuild_does_not_retry_for_the_same_sources() {
    // Otherwise a project that does not compile starts a Gradle build on every
    // single arbor invocation and never succeeds.
    let temp = setup_java_project();
    let dir = temp.path();

    run_arbor_stdout(dir, &["scip", "index.scip", "--root", "."]);
    make_index_stale(dir);

    let bin = stub_scip_java(dir, false);
    let first = run_arbor_with_path(dir, &bin, &["status", "."]);
    assert!(
        String::from_utf8_lossy(&first.stderr).contains("Rebuild failed"),
        "expected a failure notice"
    );

    let second = run_arbor_with_path(dir, &bin, &["status", "."]);
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(
        stderr.contains("last rebuild failed for these same sources"),
        "must not retry identically, got: {stderr}"
    );
    // ...and the command still answers from the cache.
    assert!(second.status.success());
}

#[test]
fn auto_rebuild_can_be_switched_off() {
    let temp = setup_java_project();
    let dir = temp.path();

    run_arbor_stdout(dir, &["scip", "index.scip", "--root", "."]);
    make_index_stale(dir);

    let bin = stub_scip_java(dir, true);
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_arbor"))
        .args(["status", "."])
        .current_dir(dir)
        .env("PATH", path)
        .env("ARBOR_NO_AUTO_REBUILD", "1")
        .output()
        .expect("run arbor");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("ARBOR_NO_AUTO_REBUILD"), "got: {stderr}");
    assert!(!stderr.contains("rebuilding now"));
    assert!(output.status.success(), "must still answer from cache");
}

#[test]
fn setup_is_not_a_dead_end_on_a_scip_project() {
    // `setup` calls `index`, which correctly refuses on a SCIP project — but
    // `setup` has no `--force` to offer, so before this it left the user with
    // an error and no way forward.
    let temp = setup_java_project();
    let dir = temp.path();

    run_arbor_stdout(dir, &["scip", "index.scip", "--root", "."]);

    let output = run_arbor(dir, &["setup", "."]);
    assert!(
        output.status.success(),
        "setup must succeed on an already-set-up SCIP project: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Already set up from a SCIP index"),
        "got: {stdout}"
    );
    assert!(
        stdout.contains("arbor scip --background"),
        "should name the refresh command: {stdout}"
    );
    assert!(
        dir.join(".arbor/scip.json").exists(),
        "setup must not clear the provenance marker"
    );
}

#[test]
fn setup_still_indexes_a_plain_project() {
    let temp = setup_java_project();
    let dir = temp.path();
    // No `arbor scip` run, so no provenance marker — Tree-sitter path.
    let stdout = run_arbor_stdout(dir, &["setup", "."]);
    assert!(stdout.contains("Indexed"), "got: {stdout}");
    assert!(!dir.join(".arbor/scip.json").exists());
}

/// `Order` also matches `OrderRequest` under grep; a compiler index does not
/// have that problem. `Checkout.gateway`'s declared type is `Gateway`, so
/// `uses-type Gateway` must find it.
#[test]
fn uses_type_finds_the_checkout_field() {
    let temp = setup_java_project_with_relationships();
    let dir = temp.path();

    run_arbor_stdout(dir, &["scip", "index.scip", "--root", "."]);

    let stdout = run_arbor_stdout(dir, &["uses-type", "Gateway", "."]);
    assert!(
        stdout.contains("com.example.Checkout.gateway"),
        "expected the field typed as Gateway, got: {stdout}"
    );
}

/// `arbor callers` reports zero for a field — it is never called — but the
/// field is still touched by `Checkout.pay`, which `references` can see.
#[test]
fn references_finds_the_field_access() {
    let temp = setup_java_project_with_relationships();
    let dir = temp.path();

    run_arbor_stdout(dir, &["scip", "index.scip", "--root", "."]);

    let stdout = run_arbor_stdout(dir, &["references", "Checkout.gateway", "."]);
    assert!(
        stdout.contains("com.example.Checkout.pay"),
        "expected pay() to show up as a reference to the field, got: {stdout}"
    );
    assert!(
        stdout.contains("read") || stdout.contains("write") || stdout.contains("access-role"),
        "must note that read vs. write is not knowable, got: {stdout}"
    );
}

/// The outgoing direction of the fixture's existing `implements` relationship:
/// `StripeGateway.charge` implements `Gateway.charge`.
#[test]
fn supertypes_finds_the_interface_method() {
    let temp = setup_java_project();
    let dir = temp.path();

    run_arbor_stdout(dir, &["scip", "index.scip", "--root", "."]);

    let stdout = run_arbor_stdout(dir, &["supertypes", "StripeGateway.charge", "."]);
    assert!(
        stdout.contains("com.example.Gateway.charge"),
        "expected the interface method it implements, got: {stdout}"
    );
}

/// The hard requirement for every one of these commands, same as
/// `implementors`: a Tree-sitter graph must say it cannot answer, never that
/// there is nothing to find.
#[test]
fn uses_type_on_a_tree_sitter_graph_says_it_cannot_answer() {
    let temp = setup_java_project_with_relationships();
    let dir = temp.path();

    run_arbor_stdout(dir, &["index", "."]);

    let output = run_arbor(dir, &["uses-type", "Gateway", "."]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "a missing capability is not a user error: {stdout}"
    );
    assert!(
        stdout.contains("cannot be answered"),
        "must say the graph cannot answer: {stdout}"
    );
    assert!(stdout.contains("Tree-sitter"), "got: {stdout}");
    assert!(stdout.contains("arbor scip"), "got: {stdout}");
    assert!(
        !stdout.contains("Nothing is typed as"),
        "must not report absence as fact: {stdout}"
    );
}

#[test]
fn references_on_a_tree_sitter_graph_says_it_cannot_answer() {
    let temp = setup_java_project_with_relationships();
    let dir = temp.path();

    run_arbor_stdout(dir, &["index", "."]);

    let output = run_arbor(dir, &["references", "Checkout.gateway", "."]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "a missing capability is not a user error: {stdout}"
    );
    assert!(
        stdout.contains("cannot be answered"),
        "must say the graph cannot answer: {stdout}"
    );
    assert!(stdout.contains("Tree-sitter"), "got: {stdout}");
    assert!(stdout.contains("arbor scip"), "got: {stdout}");
    assert!(
        !stdout.contains("Nothing references"),
        "must not report absence as fact: {stdout}"
    );
}

#[test]
fn supertypes_on_a_tree_sitter_graph_says_it_cannot_answer() {
    let temp = setup_java_project();
    let dir = temp.path();

    run_arbor_stdout(dir, &["index", "."]);

    let output = run_arbor(dir, &["supertypes", "StripeGateway.charge", "."]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "a missing capability is not a user error: {stdout}"
    );
    assert!(
        stdout.contains("no type hierarchy"),
        "must say the graph cannot answer: {stdout}"
    );
    assert!(stdout.contains("Tree-sitter"), "got: {stdout}");
    assert!(stdout.contains("arbor scip"), "got: {stdout}");
    assert!(
        !stdout.contains("implements or extends nothing"),
        "must not report absence as fact: {stdout}"
    );
}

/// The JSON form has to carry the same distinction as the human output, or a
/// tool consuming it re-derives the mistake straight from an empty list.
#[test]
fn uses_type_json_reports_whether_the_answer_is_knowable() {
    let temp = setup_java_project_with_relationships();
    let dir = temp.path();

    run_arbor_stdout(dir, &["index", "."]);
    let stdout = run_arbor_stdout(dir, &["uses-type", "Gateway", ".", "--json"]);
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");

    assert_eq!(value["usesTypeAvailable"], false);
    assert_eq!(value["provenance"], "tree-sitter");
    assert_eq!(value["usesType"].as_array().unwrap().len(), 0);
}

/// `inspect --json` on a SCIP graph must carry the relationship data at all —
/// the whole point of the new section.
#[test]
fn inspect_json_carries_relationships_on_a_scip_graph() {
    let temp = setup_java_project_with_relationships();
    let dir = temp.path();

    run_arbor_stdout(dir, &["scip", "index.scip", "--root", "."]);

    let stdout = run_arbor_stdout(dir, &["inspect", "Checkout.gateway", ".", "--json"]);
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");

    let relationships = &value["relationships"];
    assert!(
        relationships.is_object(),
        "expected a relationships object, got: {value}"
    );

    let outgoing_uses_type = &relationships["outgoing"]["uses_type"];
    assert_eq!(
        outgoing_uses_type["count"], 1,
        "the field's declared type must show up as an outgoing uses_type edge: {value}"
    );
    assert!(
        outgoing_uses_type["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["qualifiedName"].as_str().unwrap().contains("Gateway")),
        "got: {value}"
    );

    let incoming_references = &relationships["incoming"]["references"];
    assert_eq!(
        incoming_references["count"], 1,
        "pay()'s access to the field must show up as an incoming references edge: {value}"
    );
}

/// `uses-type Gateway` has two matches in this fixture — `Checkout.gateway`
/// and the test fixture's `CheckoutTest.gateway` — so `--limit 1` must both
/// cut the list and say so in numbers that agree with what is printed.
#[test]
fn uses_type_limit_truncates_and_says_so() {
    let temp = setup_java_project_with_relationships();
    let dir = temp.path();

    run_arbor_stdout(dir, &["scip", "index.scip", "--root", "."]);

    let stdout = run_arbor_stdout(dir, &["uses-type", "Gateway", ".", "--limit", "1"]);
    assert!(
        stdout.contains("2 total, showing 1"),
        "header must name both numbers, got: {stdout}"
    );
    assert!(
        stdout.contains("Showing 1 of 2"),
        "footer must name both numbers, got: {stdout}"
    );
    assert!(
        stdout.contains("--limit 0"),
        "footer must name the escape hatch, got: {stdout}"
    );
}

/// The JSON form must carry `total` so a consumer can never mistake a
/// truncated list for the whole answer.
#[test]
fn uses_type_json_carries_total_when_truncated() {
    let temp = setup_java_project_with_relationships();
    let dir = temp.path();

    run_arbor_stdout(dir, &["scip", "index.scip", "--root", "."]);

    let stdout = run_arbor_stdout(
        dir,
        &["uses-type", "Gateway", ".", "--json", "--limit", "1"],
    );
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");

    assert_eq!(value["total"], 2, "got: {value}");
    assert_eq!(
        value["usesType"].as_array().unwrap().len(),
        1,
        "the list itself must stay truncated: {value}"
    );
}

/// `--exclude-test` must drop the match that lives under `src/test/...`.
#[test]
fn uses_type_exclude_test_drops_test_file_result() {
    let temp = setup_java_project_with_relationships();
    let dir = temp.path();

    run_arbor_stdout(dir, &["scip", "index.scip", "--root", "."]);

    let stdout = run_arbor_stdout(dir, &["uses-type", "Gateway", ".", "--exclude-test"]);
    assert!(
        stdout.contains("Checkout.gateway"),
        "the non-test match must survive: {stdout}"
    );
    assert!(
        !stdout.contains("CheckoutTest"),
        "the test-file match must be filtered out: {stdout}"
    );
}
