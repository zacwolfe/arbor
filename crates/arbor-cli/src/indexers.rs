//! The SCIP indexers Arbor knows how to run.
//!
//! Ingestion is language-neutral (see `arbor-scip`), so the only per-language
//! knowledge Arbor needs is operational: what the binary is called, how to tell
//! it applies to this project, and which of its failures are worth retrying.
//! That is a table, not a hierarchy — every entry below is data, and the one
//! behavioural difference (`scip-java`'s Gradle configuration-cache retry) is a
//! function pointer rather than a subclass.
//!
//! Adding a language is one row. If a row is wrong, exactly one project type
//! breaks, and it breaks by printing an install hint rather than by guessing.

use std::path::Path;

/// Given a failed run's combined output, the extra arguments to retry with.
pub type RetryRule = fn(&str) -> Option<Vec<String>>;

/// One indexer Arbor can invoke on the user's behalf.
pub struct Indexer {
    /// Executable name, looked up on `PATH`.
    pub binary: &'static str,

    /// Human label, used in messages and in `.arbor/scip.json`.
    pub language: &'static str,

    /// Files at the project root that mean "this indexer applies".
    ///
    /// A leading `*.` matches by extension, so `*.sln` finds a solution file
    /// whatever it is called.
    pub markers: &'static [&'static str],

    /// Arguments that produce an index in the current directory.
    pub args: &'static [&'static str],

    /// Arguments that depend on the project — `scip-python` requires a project
    /// name, and there is nowhere static to put one.
    pub dynamic_args: Option<fn(&Path) -> Vec<String>>,

    /// Printed when the binary is missing. Not a link: a user who has just been
    /// told they cannot index wants the command, not a browser tab.
    pub install: &'static str,

    /// Given the failed run's combined output, the extra arguments to retry
    /// with — or `None` to accept the failure.
    ///
    /// Only `scip-java` needs this. A retry that fires on an unrelated build
    /// break would mask a compile error as a SCIP problem, so each retry must
    /// prove the failure is the one it knows how to fix.
    pub retry: Option<RetryRule>,
}

/// Every indexer Arbor can drive, in detection order.
///
/// Order matters only for the message a user reads, not for correctness: a
/// polyglot project runs every indexer that matches, and the resulting indexes
/// are ingested together so cross-language files all land in one graph.
pub const INDEXERS: &[Indexer] = &[
    Indexer {
        binary: "scip-java",
        language: "Java/Kotlin",
        markers: &["build.gradle", "build.gradle.kts", "pom.xml"],
        args: &["index"],
        dynamic_args: None,
        install: "coursier bootstrap --standalone -o scip-java \
org.scip-code:scip-java:0.13.1 --main org.scip_code.scip_java.ScipJava",
        retry: Some(crate::scip_pipeline::gradle_config_cache_retry),
    },
    Indexer {
        binary: "rust-analyzer",
        language: "Rust",
        markers: &["Cargo.toml"],
        args: &["scip", "."],
        dynamic_args: None,
        install: "rustup component add rust-analyzer",
        retry: None,
    },
    Indexer {
        binary: "scip-typescript",
        language: "TypeScript/JavaScript",
        markers: &["tsconfig.json"],
        args: &["index"],
        dynamic_args: None,
        install: "npm install -g @sourcegraph/scip-typescript",
        retry: None,
    },
    Indexer {
        binary: "scip-python",
        language: "Python",
        markers: &["pyproject.toml", "setup.py", "requirements.txt"],
        args: &["index", "."],
        // Required by scip-python, with no default. The directory name is the
        // same thing a human would type.
        dynamic_args: Some(project_name_flag),
        install: "npm install -g @sourcegraph/scip-python",
        retry: None,
    },
    Indexer {
        binary: "scip-go",
        language: "Go",
        markers: &["go.mod"],
        args: &[],
        dynamic_args: None,
        install: "go install github.com/scip-code/scip-go/cmd/scip-go@latest",
        retry: None,
    },
    Indexer {
        binary: "scip-dotnet",
        language: "C#",
        markers: &["*.sln", "*.csproj"],
        args: &["index"],
        dynamic_args: None,
        install: "dotnet tool install --global scip-dotnet",
        retry: None,
    },
    Indexer {
        binary: "scip-clang",
        language: "C/C++",
        // Deliberately the compilation database rather than CMakeLists.txt:
        // scip-clang cannot run without one, so matching on the build script
        // would promise an index Arbor cannot deliver.
        markers: &["compile_commands.json"],
        args: &["--compdb-path=compile_commands.json"],
        dynamic_args: None,
        install: "download a release from https://github.com/sourcegraph/scip-clang/releases",
        retry: None,
    },
    Indexer {
        binary: "scip-ruby",
        language: "Ruby",
        markers: &["Gemfile"],
        args: &["."],
        dynamic_args: None,
        install: "add gem 'scip-ruby' to your Gemfile's development group",
        retry: None,
    },
    Indexer {
        binary: "scip-php",
        language: "PHP",
        markers: &["composer.json"],
        args: &[],
        dynamic_args: None,
        install: "composer require --dev davidrjenni/scip-php",
        retry: None,
    },
    Indexer {
        binary: "scip-dart",
        language: "Dart",
        markers: &["pubspec.yaml"],
        args: &["."],
        dynamic_args: None,
        install: "dart pub global activate scip_dart",
        retry: None,
    },
];

/// `--project-name <dir>`, which `scip-python` requires and has no default for.
fn project_name_flag(project_root: &Path) -> Vec<String> {
    let name = project_root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "project".to_string());

    vec!["--project-name".to_string(), name]
}

impl Indexer {
    /// The full argument list for this project.
    pub fn arguments(&self, project_root: &Path) -> Vec<String> {
        let mut args: Vec<String> = self.args.iter().map(|a| (*a).to_string()).collect();
        if let Some(dynamic) = self.dynamic_args {
            args.extend(dynamic(project_root));
        }
        args
    }

    /// Whether this project looks like one this indexer handles.
    pub fn applies_to(&self, project_root: &Path) -> bool {
        self.markers
            .iter()
            .any(|marker| marker_present(project_root, marker))
    }
}

/// Whether a marker — a filename, or `*.ext` — exists at the project root.
///
/// Root-only on purpose. A `tsconfig.json` three directories down usually means
/// a sub-package that its own build already covers, and walking the tree would
/// make `arbor scip` start a build for every vendored fixture in the repo.
fn marker_present(project_root: &Path, marker: &str) -> bool {
    if let Some(extension) = marker.strip_prefix("*.") {
        let Ok(entries) = std::fs::read_dir(project_root) else {
            return false;
        };
        return entries.flatten().any(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|found| found.eq_ignore_ascii_case(extension))
        });
    }

    project_root.join(marker).exists()
}

/// Every indexer whose markers are present, whether or not it is installed.
pub fn detect(project_root: &Path) -> Vec<&'static Indexer> {
    INDEXERS
        .iter()
        .filter(|indexer| indexer.applies_to(project_root))
        .collect()
}

/// Looks an indexer up by binary name.
#[cfg(test)]
pub fn by_binary(binary: &str) -> Option<&'static Indexer> {
    INDEXERS.iter().find(|indexer| indexer.binary == binary)
}

/// Whether an executable of this name is on `PATH`.
///
/// Deliberately a `PATH` walk rather than probing with `--version`: `scip-java`
/// is a JVM launcher, so that probe would start a whole JVM just to answer "does
/// this exist", and some launchers exit non-zero on an unrecognised flag anyway.
pub fn on_path(binary: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };

    std::env::split_paths(&path).any(|dir| {
        let candidate = dir.join(binary);
        match candidate.metadata() {
            #[cfg(unix)]
            Ok(meta) => {
                use std::os::unix::fs::PermissionsExt;
                meta.is_file() && meta.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            Ok(meta) => meta.is_file(),
            Err(_) => false,
        }
    })
}

/// A one-line summary for messages: `scip-java (Java/Kotlin)`.
pub fn describe(indexers: &[&'static Indexer]) -> String {
    indexers
        .iter()
        .map(|i| format!("{} ({})", i.binary, i.language))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn project(files: &[&str]) -> TempDir {
        let dir = TempDir::new().unwrap();
        for file in files {
            fs::write(dir.path().join(file), "").unwrap();
        }
        dir
    }

    #[test]
    fn detects_a_gradle_project() {
        let dir = project(&["build.gradle.kts"]);
        let found = detect(dir.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].binary, "scip-java");
    }

    #[test]
    fn detects_nothing_in_an_empty_directory() {
        let dir = project(&[]);
        assert!(detect(dir.path()).is_empty());
    }

    #[test]
    fn detects_every_indexer_a_polyglot_repo_needs() {
        // Argus's shape: a Gradle backend with a TypeScript front end.
        let dir = project(&["build.gradle.kts", "tsconfig.json"]);
        let found: Vec<&str> = detect(dir.path()).iter().map(|i| i.binary).collect();
        assert_eq!(found, vec!["scip-java", "scip-typescript"]);
    }

    #[test]
    fn extension_markers_match_any_filename() {
        let dir = project(&["Whatever.sln"]);
        let found: Vec<&str> = detect(dir.path()).iter().map(|i| i.binary).collect();
        assert_eq!(found, vec!["scip-dotnet"]);
    }

    #[test]
    fn markers_are_root_only() {
        // A tsconfig.json in a subdirectory is somebody else's build.
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join("web")).unwrap();
        fs::write(dir.path().join("web").join("tsconfig.json"), "").unwrap();
        assert!(detect(dir.path()).is_empty());
    }

    #[test]
    fn c_and_cpp_need_a_compilation_database_not_a_build_script() {
        let cmake = project(&["CMakeLists.txt"]);
        assert!(detect(cmake.path()).is_empty());

        let compdb = project(&["compile_commands.json"]);
        assert_eq!(detect(compdb.path())[0].binary, "scip-clang");
    }

    #[test]
    fn python_gets_a_project_name() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("my-service");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("pyproject.toml"), "").unwrap();

        let indexer = by_binary("scip-python").unwrap();
        let args = indexer.arguments(&root);
        assert_eq!(
            args,
            vec!["index", ".", "--project-name", "my-service"],
            "scip-python refuses to run without --project-name"
        );
    }

    #[test]
    fn only_scip_java_retries() {
        let retrying: Vec<&str> = INDEXERS
            .iter()
            .filter(|i| i.retry.is_some())
            .map(|i| i.binary)
            .collect();
        assert_eq!(retrying, vec!["scip-java"]);
    }

    #[test]
    fn every_indexer_has_an_install_hint_and_markers() {
        for indexer in INDEXERS {
            assert!(!indexer.install.is_empty(), "{}", indexer.binary);
            assert!(!indexer.markers.is_empty(), "{}", indexer.binary);
            assert!(!indexer.language.is_empty(), "{}", indexer.binary);
        }
    }
}
