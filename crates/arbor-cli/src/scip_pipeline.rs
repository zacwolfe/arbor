//! Running SCIP indexers from Arbor, for `arbor scip --background`.
//!
//! This deliberately duplicates the retry logic in `scripts/scip-index.sh`.
//! The script has to keep working standalone — for users who would rather not
//! have Arbor spawn multi-minute Gradle builds, and for CI where the index is
//! produced in a separate step — so neither can be expressed in terms of the
//! other. The shared behaviour is small and its two copies are covered by
//! tests on both sides.
//!
//! Which indexers exist and how to invoke them is [`crate::indexers`]. This
//! module only knows how to run one, retry it if its own rule says to, and
//! collect whatever indexes it produced.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use crate::indexers::Indexer;

/// Outcome of one indexer invocation.
pub struct PipelineRun {
    pub success: bool,
    /// Combined stdout+stderr, kept whole: a failed compile is what the user
    /// actually needs to read.
    pub output: String,
    /// Whether the indexer's retry rule had to be applied.
    pub retried: bool,
}

/// Runs one indexer, applying its retry rule if the first attempt fails.
///
/// The first attempt is never skipped in favour of always passing the retry
/// arguments: on a project that does not need them it succeeds, and a second
/// full build would be pure waste.
pub fn run_one(project_root: &Path, indexer: &Indexer) -> std::io::Result<PipelineRun> {
    let base = indexer.arguments(project_root);
    let first = invoke(project_root, indexer, &base)?;
    if first.status.success() {
        return Ok(PipelineRun {
            success: true,
            output: first.combined,
            retried: false,
        });
    }

    let Some(retry) = indexer.retry else {
        return Ok(PipelineRun {
            success: false,
            output: first.combined,
            retried: false,
        });
    };

    let Some(extra) = retry(&first.combined) else {
        // The failure is not the one this indexer knows how to work around.
        // Retrying anyway would mask a compile error as a SCIP problem.
        return Ok(PipelineRun {
            success: false,
            output: first.combined,
            retried: false,
        });
    };

    let mut args = base;
    args.extend(extra);
    let second = invoke(project_root, indexer, &args)?;

    Ok(PipelineRun {
        success: second.status.success(),
        output: format!("{}\n{}", first.combined, second.combined),
        retried: true,
    })
}

/// What happened across every indexer this project needs.
pub struct Rebuild {
    /// Indexes produced by this rebuild, already collected under `.arbor/`.
    pub indexes: Vec<PathBuf>,

    /// Every indexer that ran, and whether it succeeded.
    pub ran: Vec<(&'static Indexer, PipelineRun)>,

    /// Detected but not installed. Reported rather than silently skipped: a
    /// missing indexer means part of the repository is absent from the graph,
    /// which looks identical to that code having no callers.
    pub missing: Vec<&'static Indexer>,

    /// Combined output of every invocation, for the failure log.
    pub log: String,
}

impl Rebuild {
    /// Whether the graph should be replaced.
    ///
    /// One index is enough. A repository whose Kotlin indexed and whose
    /// TypeScript did not is still better served by the Kotlin half than by the
    /// stale graph, and the per-indexer failures are reported either way.
    pub fn produced_anything(&self) -> bool {
        !self.indexes.is_empty()
    }

    /// Indexers that ran and failed.
    pub fn failures(&self) -> Vec<&'static Indexer> {
        self.ran
            .iter()
            .filter(|(_, run)| !run.success)
            .map(|(indexer, _)| *indexer)
            .collect()
    }
}

/// Runs every indexer this project needs, collecting the indexes they produce.
///
/// Sequential rather than parallel: these are compilers, they already saturate
/// the machine, and interleaved build output is unreadable when one of them
/// fails.
pub fn rebuild(project_root: &Path) -> std::io::Result<Rebuild> {
    let detected = crate::indexers::detect(project_root);
    let (available, missing): (Vec<_>, Vec<_>) = detected
        .into_iter()
        .partition(|indexer| crate::indexers::on_path(indexer.binary));

    // Previously collected indexes are cleared first so a module that no longer
    // exists cannot leave one behind to be ingested forever.
    clear_collected(project_root);

    let mut result = Rebuild {
        indexes: Vec::new(),
        ran: Vec::new(),
        missing,
        log: String::new(),
    };

    for indexer in available {
        // Recorded before the run so anything the indexer writes counts as new,
        // even a file it rewrites in place.
        let started = SystemTime::now();
        let run = run_one(project_root, indexer)?;

        result.log.push_str(&run.output);
        result.log.push('\n');

        if run.success {
            result
                .indexes
                .extend(collect_new_indexes(project_root, indexer, started)?);
        }

        result.ran.push((indexer, run));
    }

    Ok(result)
}

struct Invocation {
    status: std::process::ExitStatus,
    combined: String,
}

fn invoke(project_root: &Path, indexer: &Indexer, args: &[String]) -> std::io::Result<Invocation> {
    let output = Command::new(indexer.binary)
        .args(args)
        .current_dir(project_root)
        .output()?;

    let combined = format!(
        "$ {} {}\n{}{}",
        indexer.binary,
        args.join(" "),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    Ok(Invocation {
        status: output.status,
        combined,
    })
}

/// `scip-java`'s Gradle configuration-cache workaround.
///
/// The plugin's `WriteDependencies` task calls `Task.project` at execution
/// time, which the configuration cache forbids, and the error Gradle surfaces
/// (`Cannot get property 'dependenciesOut'`) names a symptom rather than the
/// cause. The fix is `--no-configuration-cache`, but arguments after `--`
/// *replace* the build tool's task list rather than append to it, so every task
/// has to be named — and those names are injected at runtime through an init
/// script, so they are not in `./gradlew tasks`. They can only be read back off
/// the command scip-java echoes.
pub fn gradle_config_cache_retry(output: &str) -> Option<Vec<String>> {
    if !mentions_configuration_cache(output) {
        return None;
    }

    let tasks = parse_task_list(output)?;

    let mut args = vec!["--".to_string()];
    args.extend(tasks);
    args.push("--no-configuration-cache".to_string());
    Some(args)
}

fn mentions_configuration_cache(output: &str) -> bool {
    let lower = output.to_lowercase();
    lower.contains("configuration cache") || lower.contains("configuration-cache")
}

/// Recovers the build tool's task list from the command `scip-java` echoes.
///
/// `scip-java` prints the invocation it runs, prefixed with `$`. The task list
/// is the trailing run of tokens that are neither options nor paths — the path
/// check is what rejects `--init-script`'s argument.
fn parse_task_list(output: &str) -> Option<Vec<String>> {
    let line = output
        .lines()
        .find(|l| l.trim_start().starts_with("$ ") && l.contains("gradle"))?;
    let line = line.trim_start().trim_start_matches("$ ");

    let tokens: Vec<&str> = line.split_whitespace().collect();
    let mut tasks: Vec<String> = Vec::new();

    for token in tokens.iter().rev() {
        if token.starts_with('-') || token.contains('/') {
            break;
        }
        tasks.push((*token).to_string());
    }

    tasks.reverse();
    match tasks.is_empty() {
        true => None,
        false => Some(tasks),
    }
}

/// Where collected indexes live.
///
/// Under `.arbor/` rather than wherever the indexer dropped them, for two
/// reasons: `gradlew clean` deletes `build/`, which would silently destroy the
/// provenance `.arbor/scip.json` points at, and every one of these tools writes
/// to `index.scip` by default — so two indexers in one repository would
/// overwrite each other.
pub fn collected_dir(project_root: &Path) -> PathBuf {
    project_root.join(".arbor").join("scip-indexes")
}

/// Moves the indexes an indexer just produced into `.arbor/scip-indexes/`.
///
/// Identified by modification time rather than by asking each tool where it
/// writes: `--output` is spelled differently or missing across these ten, and
/// `scip-java` emits one index per module in paths only it knows.
pub fn collect_new_indexes(
    project_root: &Path,
    indexer: &Indexer,
    since: SystemTime,
) -> std::io::Result<Vec<PathBuf>> {
    let destination = collected_dir(project_root);
    std::fs::create_dir_all(&destination)?;

    let mut collected = Vec::new();
    let slug: String = indexer
        .binary
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();

    for (position, found) in find_indexes(project_root, since).into_iter().enumerate() {
        let target = destination.join(format!("{slug}-{position}.scip"));
        // Same filesystem in the normal case; copy covers the rest.
        if std::fs::rename(&found, &target).is_err() {
            std::fs::copy(&found, &target)?;
            let _ = std::fs::remove_file(&found);
        }
        collected.push(target);
    }

    Ok(collected)
}

/// Non-empty `index.scip` files modified at or after `since`.
fn find_indexes(project_root: &Path, since: SystemTime) -> Vec<PathBuf> {
    fn walk(dir: &Path, since: SystemTime, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();

            if path.is_dir() {
                // `.arbor` is skipped so already-collected indexes are never
                // re-collected into themselves.
                if name != ".git" && name != ".arbor" && name != "node_modules" {
                    walk(&path, since, out);
                }
                continue;
            }

            if name != "index.scip" {
                continue;
            }

            let Ok(meta) = entry.metadata() else { continue };
            if meta.len() == 0 {
                continue;
            }
            match meta.modified() {
                Ok(modified) if modified >= since => out.push(path),
                _ => {}
            }
        }
    }

    let mut out = Vec::new();
    walk(project_root, since, &mut out);
    out.sort();
    out
}

/// Clears previously collected indexes.
///
/// Called before a full rebuild so a module that no longer exists cannot leave
/// a stale index behind to be ingested forever.
pub fn clear_collected(project_root: &Path) {
    let _ = std::fs::remove_dir_all(collected_dir(project_root));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    const REAL_OUTPUT: &str = "\
$ /Users/z/code/Eligibility-Service/gradlew --no-daemon --init-script /var/folders/_t/T/scip-java1101/init-script.gradle -Pkotlin.compiler.execution.strategy=in-process -Dscip.targetroot=/Users/z/code/Eligibility-Service/build/scip-targetroot clean scipPrintDependencies scipCompileAll
> Task :scipPrintDependencies FAILED
> Cannot get property 'dependenciesOut' on extra properties extension as it does not exist
Configuration cache problems found in this build.
BUILD FAILED in 5s";

    #[test]
    fn recovers_the_task_list_from_a_real_failure() {
        let tasks = parse_task_list(REAL_OUTPUT).expect("task list must be recoverable");
        assert_eq!(
            tasks,
            vec!["clean", "scipPrintDependencies", "scipCompileAll"]
        );
    }

    #[test]
    fn rejects_the_init_script_path() {
        // The path argument of --init-script must not be mistaken for a task.
        let out = "$ ./gradlew --init-script /tmp/x/init-script.gradle";
        assert!(parse_task_list(out).is_none());
    }

    #[test]
    fn keeps_colon_qualified_task_names() {
        let out = "$ ./gradlew --no-daemon :app:clean :app:scipCompileAll";
        assert_eq!(
            parse_task_list(out).unwrap(),
            vec![":app:clean", ":app:scipCompileAll"]
        );
    }

    #[test]
    fn no_build_line_means_nothing_to_reconstruct() {
        assert!(parse_task_list("error: scip-java exploded").is_none());
    }

    #[test]
    fn detects_the_configuration_cache_failure() {
        assert!(mentions_configuration_cache(REAL_OUTPUT));
        assert!(mentions_configuration_cache("Configuration Cache problems"));
        assert!(!mentions_configuration_cache(
            "error: cannot find symbol Foo"
        ));
    }

    #[test]
    fn retry_builds_the_full_task_list_with_the_flag() {
        let args = gradle_config_cache_retry(REAL_OUTPUT).unwrap();
        assert_eq!(
            args,
            vec![
                "--",
                "clean",
                "scipPrintDependencies",
                "scipCompileAll",
                "--no-configuration-cache"
            ]
        );
    }

    #[test]
    fn retry_declines_an_unrelated_build_failure() {
        // A genuine compile error must not be retried into looking like a
        // configuration-cache problem.
        let out = "$ ./gradlew clean compileJava\nerror: cannot find symbol Foo\nBUILD FAILED";
        assert!(gradle_config_cache_retry(out).is_none());
    }

    #[test]
    fn collects_only_indexes_written_by_this_run() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("old")).unwrap();
        fs::write(root.join("old").join("index.scip"), b"stale").unwrap();

        // Everything already on disk predates the run.
        let since = SystemTime::now();
        std::thread::sleep(std::time::Duration::from_millis(20));

        fs::create_dir_all(root.join("new")).unwrap();
        fs::write(root.join("new").join("index.scip"), b"fresh").unwrap();

        let indexer = crate::indexers::by_binary("scip-typescript").unwrap();
        let collected = collect_new_indexes(root, indexer, since).unwrap();

        assert_eq!(collected.len(), 1);
        assert_eq!(fs::read(&collected[0]).unwrap(), b"fresh");
        // The stale one is untouched, not swept in.
        assert!(root.join("old").join("index.scip").exists());
    }

    #[test]
    fn collected_indexes_do_not_collide_between_indexers() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let since = SystemTime::UNIX_EPOCH;

        fs::write(root.join("index.scip"), b"ts").unwrap();
        let ts = collect_new_indexes(
            root,
            crate::indexers::by_binary("scip-typescript").unwrap(),
            since,
        )
        .unwrap();

        fs::write(root.join("index.scip"), b"java").unwrap();
        let java = collect_new_indexes(
            root,
            crate::indexers::by_binary("scip-java").unwrap(),
            since,
        )
        .unwrap();

        assert_ne!(ts[0], java[0]);
        assert_eq!(fs::read(&ts[0]).unwrap(), b"ts");
        assert_eq!(fs::read(&java[0]).unwrap(), b"java");
    }

    #[test]
    fn empty_indexes_are_not_collected() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("index.scip"), b"").unwrap();
        let collected = collect_new_indexes(
            dir.path(),
            crate::indexers::by_binary("scip-go").unwrap(),
            SystemTime::UNIX_EPOCH,
        )
        .unwrap();
        assert!(collected.is_empty(), "a failed build leaves a 0-byte index");
    }

    #[test]
    fn clearing_removes_the_collected_directory() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("index.scip"), b"x").unwrap();
        collect_new_indexes(
            dir.path(),
            crate::indexers::by_binary("scip-go").unwrap(),
            SystemTime::UNIX_EPOCH,
        )
        .unwrap();
        assert!(collected_dir(dir.path()).exists());

        clear_collected(dir.path());
        assert!(!collected_dir(dir.path()).exists());
    }
}
