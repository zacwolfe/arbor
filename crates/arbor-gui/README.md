# arbor-gui

Arbor GUI is an immediate-mode graphical visualizer interface for Arbor, built using egui. It enables developers to explore and query their codebase dependency graphs, trace callers, and inspect refactoring impact pathways interactively.

## SCIP projects (JVM)

If a project's graph was built by `arbor scip`, the GUI loads that cached
compiler-resolved graph rather than re-parsing with Tree-sitter — otherwise it
would show a materially different graph from the CLI on the same repository.

Refreshing requires re-running the compiler, which the GUI will not start. If
the cache is missing it says so and points at `arbor scip --background`.
