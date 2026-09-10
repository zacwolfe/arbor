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

    /// Additional places this indexer may be found, tried after `binary`, in
    /// order:
    ///
    /// 1. an entry starting with `~/` is resolved against the user's home
    ///    directory (`dart pub global activate` and `dotnet tool install
    ///    --global` both write to a per-user tool directory that is
    ///    frequently missing from `PATH`, and that directory is not the
    ///    project's to know);
    /// 2. an entry containing a path separator is resolved against the
    ///    project root (Composer installs to the project's own
    ///    `vendor/bin`, which is never on `PATH`);
    /// 3. anything else is searched on `PATH` as an alternate name (the tool
    ///    a language's own install command produces under a name that does
    ///    not match `binary`).
    pub also: &'static [&'static str],

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
        also: &[],
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
        also: &[],
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
        also: &[],
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
        also: &[],
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
        also: &[],
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
        // `dotnet tool install --global scip-dotnet` installs into
        // `~/.dotnet/tools`, which is a per-user tool directory .NET does not
        // add to `PATH` on every install — the same dead end the pub-cache
        // path below fixes for Dart.
        also: &["~/.dotnet/tools/scip-dotnet"],
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
        also: &[],
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
        also: &[],
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
        // `composer require --dev davidrjenni/scip-php` installs to the
        // *project's own* `vendor/bin/scip-php`, per that package's
        // `composer.json` `"bin"` entry — never onto `PATH`. Without this,
        // the documented install leaves `on_path("scip-php")` failing
        // forever.
        also: &["vendor/bin/scip-php"],
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
        // `dart pub global activate scip_dart` installs an executable named
        // `scip_dart` — underscore, not hyphen — because that is the name
        // `scip-dart`'s own `pubspec.yaml` declares under `executables:`.
        // The primary name here matches what the documented install
        // actually produces; `scip-dart` (hyphen) is kept in `also` for
        // anyone who built the binary themselves and named it after the
        // package instead.
        binary: "scip_dart",
        // Pub's own install output warns that `$HOME/.pub-cache/bin` "is not
        // on your path" — so this entry, not the hyphenated name below, is
        // the difference between the documented install working and a dead
        // end. Listed first since it is what that install actually produces.
        also: &["~/.pub-cache/bin/scip_dart", "scip-dart"],
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

    /// The executable to invoke for this indexer, or `None` if it cannot be
    /// found anywhere Arbor knows to look.
    ///
    /// Tries `binary` on `PATH` first, then each entry in `also` in order,
    /// per the three cases documented on [`Self::also`]: `~/`-prefixed
    /// against the home directory, path-separator-containing against
    /// `project_root`, otherwise an alternate name on `PATH`.
    ///
    /// A `~/` entry whose home directory cannot be resolved is skipped, not
    /// treated as an error — there may be another candidate after it.
    ///
    /// This is the *only* place that answers "is this indexer runnable, and
    /// with what" — both the detection check and the actual `Command::new`
    /// go through it, so they cannot disagree.
    pub fn resolve_binary(&self, project_root: &Path) -> Option<PathBuf> {
        if on_path(self.binary) {
            return Some(PathBuf::from(self.binary));
        }

        self.also.iter().find_map(|candidate| {
            if let Some(rest) = candidate.strip_prefix("~/") {
                let path = dirs::home_dir()?.join(rest);
                return is_executable(&path).then_some(path);
            }
            if candidate.contains('/') {
                let path = project_root.join(candidate);
                return is_executable(&path).then_some(path);
            }
            on_path(candidate).then(|| PathBuf::from(*candidate))
        })
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

    std::env::split_paths(&path).any(|dir| is_executable(&dir.join(binary)))
}

/// Whether `candidate` exists and is executable.
///
/// Shared by `on_path` and [`Indexer::resolve_binary`]'s project-relative
/// check, so a `vendor/bin/scip-php` that Composer wrote but that lost its
/// executable bit is rejected the same way a missing `PATH` entry is.
fn is_executable(candidate: &Path) -> bool {
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

    /// Serializes tests below that mutate the process's `PATH`. `cargo test`
    /// runs tests concurrently by default, and `PATH` is process-global —
    /// without this, two of these tests running at once could each see the
    /// other's temporary directory.
    static PATH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Restores `PATH` to `saved` when dropped, even if the test panics
    /// mid-assertion — otherwise a failure here would corrupt `PATH` for
    /// every test that runs afterward.
    struct RestorePath(Option<std::ffi::OsString>);

    impl Drop for RestorePath {
        fn drop(&mut self) {
            match self.0.take() {
                Some(p) => std::env::set_var("PATH", p),
                None => std::env::remove_var("PATH"),
            }
        }
    }

    /// Points `PATH` at exactly `only_dir`, returning a guard that restores
    /// the previous value on drop.
    fn set_path_to_only(only_dir: &Path) -> RestorePath {
        let saved = std::env::var_os("PATH");
        std::env::set_var("PATH", only_dir);
        RestorePath(saved)
    }

    /// Serializes tests below that mutate the process's `HOME`, for the same
    /// reason as `PATH_LOCK`. Always acquired after `PATH_LOCK` in tests that
    /// need both, so two tests locking both cannot deadlock on lock order.
    static HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Restores `HOME` to `saved` when dropped, even if the test panics
    /// mid-assertion.
    struct RestoreHome(Option<std::ffi::OsString>);

    impl Drop for RestoreHome {
        fn drop(&mut self) {
            match self.0.take() {
                Some(h) => std::env::set_var("HOME", h),
                None => std::env::remove_var("HOME"),
            }
        }
    }

    /// Points `HOME` at exactly `dir`, returning a guard that restores the
    /// previous value on drop.
    fn set_home_to(dir: &Path) -> RestoreHome {
        let saved = std::env::var_os("HOME");
        std::env::set_var("HOME", dir);
        RestoreHome(saved)
    }

    /// Writes an executable shell script at `path`, creating its parent
    /// directory if needed.
    fn write_executable(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(path).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(path, perms).unwrap();
        }
    }

    #[test]
    fn dart_primary_binary_matches_its_documented_install() {
        // `dart pub global activate scip_dart` produces an executable named
        // `scip_dart` — underscore — because scip-dart's own `pubspec.yaml`
        // declares that name under `executables:`. This is not a typo to be
        // "fixed" back to a hyphen: `binary` must match what the documented
        // install command actually produces, or that command leaves Arbor
        // unable to find what it just told the user to install. `scip-dart`
        // (hyphen) still works for anyone who built the tool themselves and
        // named the binary after the package, via `also`.
        let dart = by_binary("scip_dart").expect("scip_dart must be the primary binary name");
        assert_eq!(dart.language, "Dart");
        assert!(dart.also.contains(&"scip-dart"));
    }

    #[test]
    fn a_project_relative_candidate_is_found() {
        let _guard = PATH_LOCK.lock().unwrap();
        let dir = project(&["composer.json"]);
        write_executable(&dir.path().join("vendor/bin/scip-php"));

        // An empty directory on PATH, so `scip-php` cannot be found there —
        // only the project-relative `also` entry can succeed.
        let empty = TempDir::new().unwrap();
        let _restore = set_path_to_only(empty.path());

        let php = by_binary("scip-php").unwrap();
        assert_eq!(
            php.resolve_binary(dir.path()),
            Some(dir.path().join("vendor/bin/scip-php"))
        );
    }

    #[test]
    fn a_non_executable_project_relative_candidate_is_not_installed() {
        let _guard = PATH_LOCK.lock().unwrap();
        let dir = project(&["composer.json"]);
        let target = dir.path().join("vendor/bin/scip-php");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, "not executable").unwrap();

        let empty = TempDir::new().unwrap();
        let _restore = set_path_to_only(empty.path());

        let php = by_binary("scip-php").unwrap();
        assert_eq!(php.resolve_binary(dir.path()), None);
    }

    #[test]
    fn a_home_relative_candidate_is_found() {
        // `dart pub global activate scip_dart` installs to
        // `$HOME/.pub-cache/bin`, which Pub's own output says is not on
        // `PATH` — this is the case that made the documented install a dead
        // end before `also` understood `~/`.
        let _path_guard = PATH_LOCK.lock().unwrap();
        let _home_guard = HOME_LOCK.lock().unwrap();

        let home = TempDir::new().unwrap();
        write_executable(&home.path().join(".pub-cache/bin/scip_dart"));
        let _restore_home = set_home_to(home.path());

        // Empty PATH: neither the primary name nor the hyphenated alternate
        // can be found there, so only the `~/` candidate can succeed.
        let empty = TempDir::new().unwrap();
        let _restore_path = set_path_to_only(empty.path());

        let dart = by_binary("scip_dart").unwrap();
        let project_root = TempDir::new().unwrap();
        assert_eq!(
            dart.resolve_binary(project_root.path()),
            Some(home.path().join(".pub-cache/bin/scip_dart"))
        );
    }

    #[test]
    fn a_home_relative_candidate_that_is_not_executable_is_not_installed() {
        let _path_guard = PATH_LOCK.lock().unwrap();
        let _home_guard = HOME_LOCK.lock().unwrap();

        let home = TempDir::new().unwrap();
        let target = home.path().join(".pub-cache/bin/scip_dart");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, "not executable").unwrap();
        let _restore_home = set_home_to(home.path());

        let empty = TempDir::new().unwrap();
        let _restore_path = set_path_to_only(empty.path());

        let dart = by_binary("scip_dart").unwrap();
        let project_root = TempDir::new().unwrap();
        assert_eq!(dart.resolve_binary(project_root.path()), None);
    }

    #[test]
    fn dart_also_includes_the_pub_cache_path() {
        // Without this entry, `dart pub global activate scip_dart` — the
        // exact command Arbor's own install hint prints — leaves the binary
        // somewhere Arbor never looks, because Pub does not put it on PATH.
        let dart = by_binary("scip_dart").unwrap();
        assert_eq!(dart.also[0], "~/.pub-cache/bin/scip_dart");
    }

    #[test]
    fn an_alternate_path_name_is_found_when_the_primary_is_absent() {
        let _guard = PATH_LOCK.lock().unwrap();
        let bin_dir = TempDir::new().unwrap();
        write_executable(&bin_dir.path().join("alt-name"));
        let _restore = set_path_to_only(bin_dir.path());

        let indexer = Indexer {
            binary: "primary-name-that-is-not-installed",
            also: &["alt-name"],
            language: "Test",
            markers: &[],
            weak_markers: &[],
            extensions: &[],
            args: &[],
            dynamic_args: None,
            subproject_args: None,
            install: "n/a",
            retry: None,
        };
        let project_root = TempDir::new().unwrap();

        assert_eq!(
            indexer.resolve_binary(project_root.path()),
            Some(PathBuf::from("alt-name"))
        );
    }

    #[test]
    fn every_also_entry_is_well_formed() {
        // A malformed row must fail here rather than break one project type
        // silently: an absolute path would ignore the project root, `..`
        // would walk outside it, and a backslash is not a path separator for
        // the forward-slash join `resolve_binary` does. `~/` is the one
        // permitted absolute-looking prefix — it is resolved against the
        // home directory, not the project root — so anything else starting
        // with `~` (a different user's home) or `/` is rejected.
        for indexer in INDEXERS {
            for entry in indexer.also {
                if entry.strip_prefix("~/").is_none() {
                    assert!(
                        !entry.starts_with('/'),
                        "{}: also entry {entry:?} must not be absolute",
                        indexer.binary
                    );
                    assert!(
                        !entry.starts_with('~'),
                        "{}: also entry {entry:?} must use exactly ~/ for a home-relative path",
                        indexer.binary
                    );
                }
                assert!(
                    !entry.split('/').any(|part| part == ".."),
                    "{}: also entry {entry:?} must not walk up with ..",
                    indexer.binary
                );
                assert!(
                    !entry.contains('\\'),
                    "{}: also entry {entry:?} must use forward slashes",
                    indexer.binary
                );
            }
        }
    }
}
