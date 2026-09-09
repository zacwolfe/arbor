//! Running `scip-java` from Arbor, for `arbor scip --background`.
//!
//! This deliberately duplicates the retry logic in `scripts/scip-index.sh`.
//! The script has to keep working standalone — for users who would rather not
//! have Arbor spawn multi-minute Gradle builds, and for CI where the index is
//! produced in a separate step — so neither can be expressed in terms of the
//! other. The shared behaviour is small and its two copies are covered by
//! tests on both sides.
//!
//! Note that this makes `scip-java` a runtime dependency of `arbor`, but only
//! for this one flag: plain `arbor scip <index>` still just ingests a file.

use std::path::Path;
use std::process::Command;

/// Outcome of one `scip-java` invocation.
pub struct PipelineRun {
    pub success: bool,
    /// Combined stdout+stderr, kept whole: a failed compile is what the user
    /// actually needs to read.
    pub output: String,
    /// Whether the configuration-cache workaround had to be applied.
    pub retried: bool,
}

/// Runs `scip-java index`, retrying with `--no-configuration-cache` if the
/// first attempt died on Gradle's configuration cache.
///
/// The first attempt is not skipped in favour of always passing the flag: on a
/// project without the cache it succeeds, and a second full build would be
/// pure waste.
pub fn run(project_root: &Path) -> std::io::Result<PipelineRun> {
    let first = invoke(project_root, &[])?;
    if first.status.success() {
        return Ok(PipelineRun {
            success: true,
            output: first.combined,
            retried: false,
        });
    }

    let Some(tasks) = parse_task_list(&first.combined) else {
        return Ok(PipelineRun {
            success: false,
            output: first.combined,
            retried: false,
        });
    };

    if !mentions_configuration_cache(&first.combined) {
        // Some other build failure. Adding the flag would mask a compile error
        // as a scip problem.
        return Ok(PipelineRun {
            success: false,
            output: first.combined,
            retried: false,
        });
    }

    // Args after `--` REPLACE the build tool's task list rather than append to
    // it, so every task has to be named.
    let mut args: Vec<String> = vec!["--".to_string()];
    args.extend(tasks);
    args.push("--no-configuration-cache".to_string());

    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let second = invoke(project_root, &arg_refs)?;

    Ok(PipelineRun {
        success: second.status.success(),
        output: format!("{}\n{}", first.combined, second.combined),
        retried: true,
    })
}

struct Invocation {
    status: std::process::ExitStatus,
    combined: String,
}

fn invoke(project_root: &Path, extra: &[&str]) -> std::io::Result<Invocation> {
    let output = Command::new("scip-java")
        .arg("index")
        .args(extra)
        .current_dir(project_root)
        .output()?;

    let combined = format!(
        "$ scip-java index {}\n{}{}",
        extra.join(" "),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    Ok(Invocation {
        status: output.status,
        combined,
    })
}

fn mentions_configuration_cache(output: &str) -> bool {
    let lower = output.to_lowercase();
    lower.contains("configuration cache") || lower.contains("configuration-cache")
}

/// Recovers the build tool's task list from the command `scip-java` echoes.
///
/// `scip-java` prints the invocation it runs, prefixed with `$`. The task list
/// is the trailing run of tokens that are neither options nor paths — the path
/// check is what rejects `--init-script`'s argument. Task names are not
/// discoverable any other way: `scip-java` injects them at runtime through an
/// init script, so they never appear in `./gradlew tasks`.
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
