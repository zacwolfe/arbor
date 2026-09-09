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

use std::path::{Path, PathBuf};

/// Given a failed run's combined output, the extra arguments to retry with.
pub type RetryRule = fn(&str) -> Option<Vec<String>>;

/// One indexer Arbor can invoke on the user's behalf.
///
/// `PartialEq`/`Eq` compare by binary name: every entry is a distinct tool, and
/// comparing function pointers is neither meaningful nor stable.
#[derive(Debug)]
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

    /// Markers that only count when this indexer's own source files sit beside
    /// them.
    ///
    /// `.python-version` is a *tooling* file: pyenv pins an interpreter for a
    /// repository whose Python is eight helper scripts in `scripts/`, which is
    /// not a Python project. Requiring a `.py` in the same directory keeps a
    /// pyenv-managed script collection detected while a Kotlin monorepo stops
    /// being offered a Python index it has no use for.
    pub weak_markers: &'static [&'static str],

    /// Source extensions this indexer covers. Corroborates [`Self::weak_markers`],
    /// and is a last resort when no marker anywhere matched.
    pub extensions: &'static [&'static str],

    /// Arguments that produce an index in the current directory.
    pub args: &'static [&'static str],

    /// Arguments that depend on the project — `scip-python` requires a project
    /// name, and there is nowhere static to put one.
    pub dynamic_args: Option<fn(&Path) -> Vec<String>>,

    /// How to index a module in a subdirectory *from the repository root*,
    /// given that directory relative to the root.
    ///
    /// `None` means Arbor will not drive this indexer for a submodule. That is a
    /// deliberate refusal rather than an oversight: an indexer run inside the
    /// subdirectory emits paths relative to *it*, and ingesting those against
    /// the repository root produces a graph full of files that do not exist —
    /// which breaks `arbor diff`, `file-graph`, and every "now read this file"
    /// follow-up, silently. Only set this where the root-relative behaviour has
    /// been verified against a real index.
    pub subproject_args: Option<fn(&Path) -> Vec<String>>,

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
        weak_markers: &[],
        extensions: &["java", "kt", "kts"],
        args: &["index"],
        dynamic_args: None,
        subproject_args: None,
        install: "coursier bootstrap --standalone -o scip-java \
org.scip-code:scip-java:0.13.1 --main org.scip_code.scip_java.ScipJava",
        retry: Some(crate::scip_pipeline::gradle_config_cache_retry),
    },
    Indexer {
        binary: "rust-analyzer",
        language: "Rust",
        markers: &["Cargo.toml"],
        weak_markers: &[],
        extensions: &["rs"],
        args: &["scip", "."],
        dynamic_args: None,
        subproject_args: None,
        install: "rustup component add rust-analyzer",
        retry: None,
    },
    Indexer {
        binary: "scip-typescript",
        language: "TypeScript/JavaScript",
        markers: &["tsconfig.json"],
        weak_markers: &[],
        extensions: &["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"],
        args: &["index"],
        dynamic_args: None,
        // `scip-typescript index ui` run from the repository root emits paths
        // prefixed `ui/`, which is what makes a monorepo's front end compose
        // with its backend in one graph. Verified on a 216-document Next.js
        // module inside a Gradle repository.
        subproject_args: Some(typescript_subproject),
        install: "npm install -g @sourcegraph/scip-typescript",
        retry: None,
    },
    Indexer {
        binary: "scip-python",
        language: "Python",
        markers: &[
            "pyproject.toml",
            "setup.py",
            "setup.cfg",
            "requirements.txt",
            "Pipfile",
            "poetry.lock",
        ],
        weak_markers: &[".python-version"],
        extensions: &["py", "pyi"],
        args: &["index", "."],
        // Required by scip-python, with no default. The directory name is the
        // same thing a human would type.
        dynamic_args: Some(project_name_flag),
        // `scip-python index scripts --project-name scripts` from the repository
        // root emits paths prefixed `scripts/`. Verified against a real module
        // inside a Gradle repository.
        subproject_args: Some(python_subproject),
        install: "npm install -g @sourcegraph/scip-python",
        retry: None,
    },
    Indexer {
        binary: "scip-go",
        language: "Go",
        markers: &["go.mod"],
        weak_markers: &[],
        extensions: &["go"],
        args: &[],
        dynamic_args: None,
        subproject_args: None,
        install: "go install github.com/scip-code/scip-go/cmd/scip-go@latest",
        retry: None,
    },
    Indexer {
        binary: "scip-dotnet",
        language: "C#",
        markers: &["*.sln", "*.csproj"],
        weak_markers: &[],
        extensions: &["cs"],
        args: &["index"],
        dynamic_args: None,
        subproject_args: None,
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
        weak_markers: &[],
        extensions: &[],
        args: &["--compdb-path=compile_commands.json"],
        dynamic_args: None,
        subproject_args: None,
        install: "download a release from https://github.com/sourcegraph/scip-clang/releases",
        retry: None,
    },
    Indexer {
        binary: "scip-ruby",
        language: "Ruby",
        markers: &["Gemfile"],
        weak_markers: &[],
        extensions: &["rb"],
        args: &["."],
        dynamic_args: None,
        subproject_args: None,
        install: "add gem 'scip-ruby' to your Gemfile's development group",
        retry: None,
    },
    Indexer {
        binary: "scip-php",
        language: "PHP",
        markers: &["composer.json"],
        weak_markers: &[],
        extensions: &["php"],
        args: &[],
        dynamic_args: None,
        subproject_args: None,
        install: "composer require --dev davidrjenni/scip-php",
        retry: None,
    },
    Indexer {
        binary: "scip-dart",
        language: "Dart",
        markers: &["pubspec.yaml"],
        weak_markers: &[],
        extensions: &["dart"],
        args: &["."],
        dynamic_args: None,
        subproject_args: None,
        install: "dart pub global activate scip_dart",
        retry: None,
    },
];

/// `scip-typescript index <dir>`, run from the repository root.
fn typescript_subproject(module: &Path) -> Vec<String> {
    vec!["index".to_string(), module.to_string_lossy().to_string()]
}

/// `scip-python index <dir> --project-name <dir>`, run from the repository root.
fn python_subproject(module: &Path) -> Vec<String> {
    let name = module
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "module".to_string());

    vec![
        "index".to_string(),
        module.to_string_lossy().to_string(),
        "--project-name".to_string(),
        name,
    ]
}

/// `--project-name <dir>`, which `scip-python` requires and has no default for.
fn project_name_flag(project_root: &Path) -> Vec<String> {
    let name = project_root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "project".to_string());

    vec!["--project-name".to_string(), name]
}

impl PartialEq for Indexer {
    fn eq(&self, other: &Self) -> bool {
        self.binary == other.binary
    }
}

impl Eq for Indexer {}

/// An indexer paired with the module it was detected for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detected {
    pub indexer: &'static Indexer,

    /// `None` for the repository root, otherwise the module directory relative
    /// to it — `ui` for a Next.js front end inside a Gradle repository.
    pub module: Option<PathBuf>,
}

impl Detected {
    /// What to print for this module: `scip-typescript (TypeScript/JavaScript)`,
    /// or `scip-typescript (TypeScript/JavaScript in ui/)`.
    pub fn label(&self) -> String {
        match &self.module {
            None => format!("{} ({})", self.indexer.binary, self.indexer.language),
            Some(dir) => format!(
                "{} ({} in {}/)",
                self.indexer.binary,
                self.indexer.language,
                dir.display()
            ),
        }
    }

    /// Whether Arbor can actually drive this one.
    ///
    /// A submodule needs an indexer that emits root-relative paths when given a
    /// directory; without that the graph would name files that do not exist.
    pub fn is_runnable(&self) -> bool {
        self.module.is_none() || self.indexer.subproject_args.is_some()
    }

    /// The argument list for this module.
    pub fn arguments(&self, project_root: &Path) -> Option<Vec<String>> {
        match &self.module {
            None => Some(self.indexer.arguments(project_root)),
            Some(dir) => self.indexer.subproject_args.map(|build| build(dir)),
        }
    }
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
        if self
            .markers
            .iter()
            .any(|marker| marker_present(project_root, marker))
        {
            return true;
        }

        self.weak_markers
            .iter()
            .any(|marker| marker_present(project_root, marker))
            && self.has_source_here(project_root)
    }

    /// Whether any of this indexer's source files sit directly in `directory`.
    fn has_source_here(&self, directory: &Path) -> bool {
        let present = root_extensions(directory);
        self.extensions
            .iter()
            .any(|ext| present.iter().any(|found| found == ext))
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

/// Every module Arbor should index, whether or not its indexer is installed.
///
/// Three layers, in order, and only the first that produces anything is used:
///
///   1. markers at the repository root
///   2. markers in *immediate* subdirectories — a monorepo's `ui/tsconfig.json`
///      beside a root `build.gradle.kts` is the common shape, and root-only
///      detection made that front end invisible
///   3. the source extensions of the files at the root, for a project with no
///      manifest at all
///
/// Depth one, deliberately. Recursing would find every vendored fixture, sample
/// app and generated bundle in the tree and offer to start an indexer for each.
pub fn detect(project_root: &Path) -> Vec<Detected> {
    let mut found: Vec<Detected> = INDEXERS
        .iter()
        .filter(|indexer| indexer.applies_to(project_root))
        .map(|indexer| Detected {
            indexer,
            module: None,
        })
        .collect();

    for module in module_directories(project_root) {
        for indexer in INDEXERS {
            // An indexer already detected at the root covers its own modules:
            // Gradle and Maven resolve a multi-module build themselves, and
            // running a second pass over one subdirectory would duplicate work
            // and fight the first for the build lock.
            if found.iter().any(|d| std::ptr::eq(d.indexer, indexer)) {
                continue;
            }
            if indexer.applies_to(&project_root.join(&module)) {
                found.push(Detected {
                    indexer,
                    module: Some(module.clone()),
                });
            }
        }
    }

    if !found.is_empty() {
        return found;
    }

    let extensions = root_extensions(project_root);
    INDEXERS
        .iter()
        .filter(|indexer| {
            indexer
                .extensions
                .iter()
                .any(|ext| extensions.iter().any(|found| found == ext))
        })
        .map(|indexer| Detected {
            indexer,
            module: None,
        })
        .collect()
}

/// Immediate subdirectories that could hold a module, sorted for determinism.
///
/// Skips the directories that hold *other people's* code or build output. A
/// `tsconfig.json` under `node_modules` describes a dependency, and indexing it
/// would take minutes to produce a graph of somebody else's library.
fn module_directories(project_root: &Path) -> Vec<PathBuf> {
    const SKIP: &[&str] = &[
        "node_modules",
        "build",
        "target",
        "dist",
        "out",
        "vendor",
        "venv",
        "__pycache__",
        "bin",
        "obj",
        "coverage",
        "fixtures",
        "testdata",
        "examples",
    ];

    let Ok(entries) = std::fs::read_dir(project_root) else {
        return Vec::new();
    };

    let mut directories: Vec<PathBuf> = entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name())
        .filter(|name| {
            let name = name.to_string_lossy();
            // Hidden directories are tooling, not modules.
            !name.starts_with('.') && !SKIP.contains(&name.as_ref())
        })
        .map(PathBuf::from)
        .collect();

    directories.sort();
    directories
}

/// Extensions of the files directly inside the project root.
///
/// Root-only, like the markers, and for the same reason: a repository's own
/// language is visible at its top level, while walking the tree would find every
/// vendored fixture and test asset.
fn root_extensions(project_root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(project_root) else {
        return Vec::new();
    };

    let mut extensions: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.path().is_file())
        .filter_map(|entry| {
            entry
                .path()
                .extension()
                .map(|ext| ext.to_string_lossy().to_lowercase())
        })
        .collect();

    extensions.sort();
    extensions.dedup();
    extensions
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

/// A one-line summary for messages: `scip-java (Java/Kotlin), scip-typescript
/// (TypeScript/JavaScript in ui/)`.
pub fn describe(detected: &[Detected]) -> String {
    detected
        .iter()
        .map(Detected::label)
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
        assert_eq!(found[0].indexer.binary, "scip-java");
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
        let found: Vec<&str> = detect(dir.path())
            .iter()
            .map(|d| d.indexer.binary)
            .collect();
        assert_eq!(found, vec!["scip-java", "scip-typescript"]);
    }

    #[test]
    fn extension_markers_match_any_filename() {
        let dir = project(&["Whatever.sln"]);
        let found: Vec<&str> = detect(dir.path())
            .iter()
            .map(|d| d.indexer.binary)
            .collect();
        assert_eq!(found, vec!["scip-dotnet"]);
    }

    /// Argus's shape: a Gradle backend at the root, a Next.js front end in ui/.
    /// Root-only detection made those 213 TypeScript files invisible.
    #[test]
    fn a_module_in_a_subdirectory_is_detected() {
        let dir = project(&["build.gradle.kts"]);
        fs::create_dir(dir.path().join("ui")).unwrap();
        fs::write(dir.path().join("ui").join("tsconfig.json"), "").unwrap();

        let found = detect(dir.path());
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].indexer.binary, "scip-java");
        assert_eq!(found[0].module, None);
        assert_eq!(found[1].indexer.binary, "scip-typescript");
        assert_eq!(found[1].module.as_deref(), Some(Path::new("ui")));
        assert_eq!(
            found[1].label(),
            "scip-typescript (TypeScript/JavaScript in ui/)"
        );
    }

    #[test]
    fn a_dependency_directory_is_not_a_module() {
        // A tsconfig.json under node_modules describes somebody else's library.
        let dir = project(&["build.gradle.kts"]);
        for skipped in ["node_modules", "build", "dist", ".venv", "examples"] {
            fs::create_dir(dir.path().join(skipped)).unwrap();
            fs::write(dir.path().join(skipped).join("tsconfig.json"), "").unwrap();
        }

        let found = detect(dir.path());
        assert_eq!(found.len(), 1, "only the root Gradle build should be found");
        assert_eq!(found[0].indexer.binary, "scip-java");
    }

    #[test]
    fn a_build_tool_detected_at_the_root_owns_its_own_submodules() {
        // Gradle resolves a multi-module build itself; a second pass over one
        // module would duplicate work and fight the first for the build lock.
        let dir = project(&["build.gradle.kts"]);
        fs::create_dir(dir.path().join("service")).unwrap();
        fs::write(dir.path().join("service").join("build.gradle.kts"), "").unwrap();

        let found = detect(dir.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].module, None);
    }

    #[test]
    fn a_python_subproject_carries_its_project_name() {
        let dir = project(&["build.gradle.kts"]);
        fs::create_dir(dir.path().join("scripts")).unwrap();
        fs::write(dir.path().join("scripts").join("requirements.txt"), "").unwrap();

        let python = detect(dir.path())
            .into_iter()
            .find(|d| d.indexer.binary == "scip-python")
            .unwrap();

        assert_eq!(
            python.arguments(dir.path()).unwrap(),
            vec!["index", "scripts", "--project-name", "scripts"]
        );
    }

    #[test]
    fn only_subdirectories_arbor_can_index_correctly_are_runnable() {
        // scip-go run inside a subdirectory emits paths relative to it, which
        // would name files that do not exist at the repository root. Detected,
        // reported, not run.
        let dir = project(&["build.gradle.kts"]);
        fs::create_dir(dir.path().join("agent")).unwrap();
        fs::write(dir.path().join("agent").join("go.mod"), "").unwrap();

        let found = detect(dir.path());
        let go = found
            .iter()
            .find(|d| d.indexer.binary == "scip-go")
            .expect("go module detected");
        assert!(!go.is_runnable());
        assert!(go.arguments(dir.path()).is_none());
    }

    #[test]
    fn a_subproject_is_indexed_from_the_repository_root() {
        let dir = project(&["build.gradle.kts"]);
        fs::create_dir(dir.path().join("ui")).unwrap();
        fs::write(dir.path().join("ui").join("tsconfig.json"), "").unwrap();

        let ts = detect(dir.path())
            .into_iter()
            .find(|d| d.indexer.binary == "scip-typescript")
            .unwrap();

        // `scip-typescript index ui`, not `cd ui && scip-typescript index` —
        // that is what makes the emitted paths root-relative.
        assert_eq!(ts.arguments(dir.path()).unwrap(), vec!["index", "ui"]);
    }

    #[test]
    fn c_and_cpp_need_a_compilation_database_not_a_build_script() {
        let cmake = project(&["CMakeLists.txt"]);
        assert!(detect(cmake.path()).is_empty());

        let compdb = project(&["compile_commands.json"]);
        assert_eq!(detect(compdb.path())[0].indexer.binary, "scip-clang");
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
    fn a_pyenv_project_with_no_manifest_is_detected() {
        // zep_config's shape: 80 .py files, no pyproject.toml, no requirements
        // — only pyenv's .python-version.
        let dir = project(&[".python-version", "billing.py", "retrieval.py"]);
        let found: Vec<&str> = detect(dir.path())
            .iter()
            .map(|d| d.indexer.binary)
            .collect();
        assert_eq!(found, vec!["scip-python"]);
    }

    #[test]
    fn source_files_alone_are_enough_when_nothing_else_matches() {
        // A bare directory of scripts, not even a .python-version.
        let dir = project(&["one.py", "two.py", "notes.md"]);
        let found: Vec<&str> = detect(dir.path())
            .iter()
            .map(|d| d.indexer.binary)
            .collect();
        assert_eq!(found, vec!["scip-python"]);
    }

    #[test]
    fn the_extension_fallback_never_fires_alongside_a_marker() {
        // A Rust project with a helper script at the root must not also try to
        // index Python: Cargo.toml already answered the question.
        let dir = project(&["Cargo.toml", "release.py"]);
        let found: Vec<&str> = detect(dir.path())
            .iter()
            .map(|d| d.indexer.binary)
            .collect();
        assert_eq!(found, vec!["rust-analyzer"]);
    }

    #[test]
    fn c_source_alone_is_not_enough() {
        // scip-clang cannot run without a compilation database, so a .cpp at the
        // root must not promise an index it cannot deliver.
        let dir = project(&["main.cpp", "util.h"]);
        assert!(detect(dir.path()).is_empty());
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
