//! Claude Code harness: CLAUDE.md directives + `.claude/settings.json` hooks +
//! arbor command permissions.

use super::{Harness, Result, Scope};
use colored::Colorize;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

/// Markers wrapping the Arbor block in CLAUDE.md so re-running `arbor hook`
/// replaces it in place instead of appending a duplicate.
const BEGIN_MARKER: &str =
    "<!-- BEGIN arbor-claude-guidance (auto-installed by `arbor hook claude`; remove this block to disable) -->";
const END_MARKER: &str = "<!-- END arbor-claude-guidance -->";

/// arbor commands allow-listed so the agent runs them without a prompt.
const PERMISSIONS: &[&str] = &[
    "Bash(arbor query *)",
    "Bash(arbor file-graph *)",
    "Bash(arbor callers *)",
    "Bash(arbor callees *)",
    // The relationship queries a call graph alone cannot answer. Read-only, and
    // the reason an agent stops guessing from grep: implementations do not call
    // the interface they implement, and a type use is not a call.
    "Bash(arbor implementors *)",
    "Bash(arbor subclasses *)",
    "Bash(arbor supertypes *)",
    "Bash(arbor uses-type *)",
    "Bash(arbor references *)",
    "Bash(arbor map *)",
    "Bash(arbor path *)",
    "Bash(arbor inspect *)",
    "Bash(arbor diff *)",
    "Bash(arbor entry-points *)",
    "Bash(arbor refactor *)",
    "Bash(arbor export *)",
    "Bash(arbor status *)",
    // Polling a rebuild is read-only. Deliberately NOT `arbor scip *`: that
    // would let an agent kick off a multi-minute Gradle build unprompted.
    "Bash(arbor scip --task-status *)",
    "Bash(arbor scip --task-status)",
];

pub struct Claude;

impl Harness for Claude {
    fn apply(&self, scope: &Scope) -> Result<()> {
        let root = match scope {
            Scope::Project(p) => p.clone(),
            Scope::Global => dirs::home_dir().ok_or("could not resolve home directory")?,
        };

        apply_directives(&root, scope)?;
        apply_settings(&root)?;

        println!(
            "{} Arbor wired into Claude Code at {}",
            "✓".green(),
            root.display()
        );
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// CLAUDE.md directives
// ---------------------------------------------------------------------------

/// Locate the CLAUDE.md to edit (root preferred, then `.claude/`), or pick the
/// default location to create one.
fn claude_md_path(root: &Path, scope: &Scope) -> PathBuf {
    match scope {
        // Global config conventionally lives at ~/.claude/CLAUDE.md.
        Scope::Global => root.join(".claude").join("CLAUDE.md"),
        Scope::Project(_) => {
            let at_root = root.join("CLAUDE.md");
            if at_root.exists() {
                return at_root;
            }
            let in_dir = root.join(".claude").join("CLAUDE.md");
            if in_dir.exists() {
                return in_dir;
            }
            // Neither exists — create at project root (the common convention).
            at_root
        }
    }
}

fn apply_directives(root: &Path, scope: &Scope) -> Result<()> {
    let path = claude_md_path(root, scope);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let existing = fs::read_to_string(&path).unwrap_or_default();
    let updated = upsert_block(&existing, &directives_block());

    if updated == existing {
        println!("  {} CLAUDE.md already up to date", "•".dimmed());
        return Ok(());
    }

    let verb = if existing.is_empty() {
        "created"
    } else {
        "updated"
    };
    fs::write(&path, updated)?;
    println!("  {} {} {}", "✓".green(), verb, path.display());
    Ok(())
}

/// Replace the existing marker-delimited Arbor block, or append a fresh one. A
/// brand-new file gets a `# CLAUDE.md` header before the block.
fn upsert_block(existing: &str, block: &str) -> String {
    if let (Some(start), Some(end)) = (existing.find(BEGIN_MARKER), existing.find(END_MARKER)) {
        let end = end + END_MARKER.len();
        let mut out = String::with_capacity(existing.len());
        out.push_str(&existing[..start]);
        out.push_str(block);
        out.push_str(&existing[end..]);
        return out;
    }

    if existing.is_empty() {
        return format!("# CLAUDE.md\n\n{block}\n");
    }

    let mut out = existing.to_string();
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push('\n');
    out.push_str(block);
    out.push('\n');
    out
}

fn directives_block() -> String {
    // The arbor guidance mirrors the reference CLAUDE.md, wrapped in markers so
    // re-running the command updates it in place.
    format!("{BEGIN_MARKER}\n{GUIDANCE}{END_MARKER}")
}

const GUIDANCE: &str = r#"## Code Navigation (MANDATORY)

This project is indexed by Arbor. **You MUST use arbor for all codebase exploration.** grep, rg, and find are blocked by hooks and will fail.

### Rules

1. **NEVER use `rg` for anything.** It is blocked. Use `arbor query "term" .` instead.
2. **NEVER use recursive grep** (`grep -r`, `grep -R`, piped from find). Use `arbor query` or `arbor callers`.
3. **NEVER use `find . -name`** to discover files. Use `arbor query "pattern" .` or `arbor file-graph "path" .`.
4. **Allowed grep**: `grep "pattern" /explicit/single/file.java` — only on a specific, known file you've already identified via arbor.
5. **NEVER Read a file to "explore" it.** Use `arbor file-graph "path" .` first to see structure, THEN Read the specific line range.

### Decision tree

| Intent | Command |
|--------|---------|
| "Where is X defined?" | `arbor query "X" . --exclude-test` |
| "What calls X?" | `arbor callers "X" .` |
| "What does X call?" | `arbor callees "X" .` |
| "Who implements this interface?" | `arbor implementors "X" .` |
| "What does this class extend?" | `arbor supertypes "X" .` |
| "Where is this type used?" | `arbor uses-type "X" .` |
| "Who touches this field or constant?" | `arbor references "X" .` |
| "What's in this file?" | `arbor file-graph "path" .` |
| "How does A connect to B?" | `arbor path "A" "B" .` |
| "What changed?" | `arbor diff .` |
| "Multi-term search" | `arbor query "term1\|term2" . --exclude-test` |

### Project map (auto-injected)

A ranked project skeleton is automatically injected into context on your first tool call each day via a PostToolUse hook. You do NOT need to run it manually — it arrives as output from your first Bash call.

The map shows the most important symbols sorted by PageRank centrality. Entry points are marked with ★. Use it to orient before diving deeper.

To manually re-run with different options:
```bash
arbor map . --exclude-test                    # default: 1024 token budget
arbor map . --exclude-test --tokens 2048      # more detail
arbor map . --exclude-test --focus "service"  # boost symbols in service layer
arbor map . --exclude-test --focus-changed    # boost symbols in files you're editing
```

### Finding symbols
- `arbor query "name" . --exclude-test` — fuzzy search for symbols (production code only)
- `arbor query "term1|term2|term3" . --exclude-test` — multi-term OR search (replaces grep with alternation)
- `arbor inspect "symbol" .` — full detail on one symbol (file, lines, role, centrality, caller/callee counts)

### Understanding relationships
- `arbor callers "symbol" .` — who calls this? (one hop upstream)
- `arbor callees "symbol" .` — what does this call? (one hop downstream)
- `arbor path "start" "end" .` — shortest path between two symbols in the call graph
- `arbor implementors "symbol" .` — who implements or extends this? Ask BEFORE changing an interface method: its implementations do not call it, so `callers` will not find them
- `arbor supertypes "symbol" .` — what does this type implement or extend?
- `arbor uses-type "symbol" .` — where does this type appear (field, parameter, return, generic)? This is the question grep answers worst: `Order` also matches `OrderRequest`
- `arbor references "symbol" .` — who touches this field, constant, or enum member? These are not calls, so a symbol with zero callers may still have dozens of references

These four need a graph built from a compiler index. On a Tree-sitter graph they say so explicitly — an empty result is **never** evidence there are none unless the command says the relationship kind is available.

### Structural views
- `arbor file-graph "src/path/File.java" .` — all symbols + internal edges within a file
- `arbor entry-points .` — list HTTP handlers, main functions, webhooks, jobs

### Impact analysis
- `arbor diff . --json` — blast radius of current git changes
- `arbor refactor "symbol" .` — blast radius of changing a specific symbol

### All commands support `--json` for structured output.

**Important:** Do NOT redirect stderr with `2>/dev/null` on arbor commands. First-run indexing logs go to stderr — suppressing them makes it look like the command is hanging.

### Workflow

1. **Orient**: `arbor map . --exclude-test` — understand the project structure
2. **Locate**: `arbor query "name" . --exclude-test` — find specific symbols
3. **Navigate**: `arbor callers`/`callees`/`path` — trace relationships
4. **Read**: Only use `Read` AFTER arbor has identified the specific file and line range you need

When `arbor query` returns test files but you need production code, do NOT fall back to grep. Instead:
1. Pick a symbol from the results (e.g., a test field or builder method)
2. Run `arbor callers "symbol" .` to trace upstream into production code
3. Or run `arbor file-graph "src/main/..." .` if you already know the production file path

### If this project uses a SCIP index (compiler-produced, any language)

`arbor status .` prints `Source: SCIP index (...)` when it does. On such a project the graph comes from the compiler, not from Tree-sitter, so `obj.method()` calls and interface implementations are real edges rather than absent ones.

Three rules follow:

1. **Never run `arbor index`.** It is refused on these projects, because Tree-sitter cannot resolve method calls and would replace exact edges with guesses. The refusal mentions `--force`; do NOT use it.
2. **Refreshing requires a compile, so it is the human's call.** If a query looks stale, say so and suggest they run `arbor scip --background`. Do not run it yourself — it starts a multi-minute Gradle build.
3. **A rebuild in flight is pollable**: `arbor scip --task-status` is read-only and safe to run.

If a read command prints `Sources changed ... rebuilding now`, it is compiling before answering. Let it finish rather than interrupting.

"#;

// ---------------------------------------------------------------------------
// .claude/settings.json — hooks + permissions
// ---------------------------------------------------------------------------

fn settings_path(root: &Path) -> PathBuf {
    root.join(".claude").join("settings.json")
}

fn apply_settings(root: &Path) -> Result<()> {
    let path = settings_path(root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut settings: Value = match fs::read_to_string(&path) {
        Ok(text) if !text.trim().is_empty() => serde_json::from_str(&text)
            .map_err(|e| format!("{} is not valid JSON: {e}", path.display()))?,
        _ => json!({}),
    };

    if !settings.is_object() {
        settings = json!({});
    }

    let hooks_changed = ensure_hooks(&mut settings);
    let perms_changed = ensure_permissions(&mut settings);

    if !hooks_changed && !perms_changed {
        println!("  {} settings.json already up to date", "•".dimmed());
        return Ok(());
    }

    fs::write(&path, serde_json::to_string_pretty(&settings)? + "\n")?;
    println!("  {} settings written to {}", "✓".green(), path.display());
    Ok(())
}

/// Arbor's three hook commands (bare `arbor`, matching the reference install).
fn arbor_hook_commands() -> (String, String, String) {
    // PreToolUse #1: auto-init .arbor/ if an arbor command runs before setup.
    let init = "echo \"$CLAUDE_TOOL_INPUT\" | grep -q '\\barbor\\b' && \
[ ! -d .arbor ] && arbor init . >/dev/null 2>&1; exit 0"
        .to_string();
    // PreToolUse #2: block rg, recursive grep, and find -name; steer to arbor.
    let block = "CMD=$(echo \"$CLAUDE_TOOL_INPUT\" | jq -r '.command // .input // .'); \
echo \"$CMD\" | grep -qE '\\brg\\b' && \
echo 'BLOCK: Use arbor query \"symbol\" . instead of rg. \
Multi-term: arbor query \"term1|term2\" . --exclude-test' && exit 1; \
echo \"$CMD\" | grep -qE '\\bgrep\\b' && \
echo \"$CMD\" | grep -qE '(-[a-zA-Z]*r|-[a-zA-Z]*R|--recursive|\\*\\*/|\\.\\.?/)' && \
echo 'BLOCK: Use arbor query/callers/callees instead of recursive grep. \
Grep on a single known file is OK.' && exit 1; \
echo \"$CMD\" | grep -qE 'find\\s+\\..*(-name|-type)' && \
echo 'BLOCK: Use arbor query \"pattern\" . to find files/symbols. \
Use arbor file-graph for file contents.' && exit 1; exit 0"
        .to_string();
    // PostToolUse: inject the project skeleton once per day.
    // ARBOR_NO_AUTO_REBUILD is essential here, not incidental. On a project
    // whose graph came from a SCIP index, a read command with stale sources
    // rebuilds synchronously — which for a JVM repo means a full compile. That
    // would stall the agent's first tool call of the day for minutes inside a
    // hook, with `2>/dev/null` hiding any explanation. The skeleton is worth
    // having fast and slightly stale; it is not worth blocking on.
    let map = "FLAG=\".arbor/.map-injected-$(date +%Y%m%d)\"; \
[ -f \"$FLAG\" ] && exit 0; touch \"$FLAG\"; \
echo '--- arbor map (project skeleton) ---'; \
ARBOR_NO_AUTO_REBUILD=1 arbor map . --exclude-test 2>/dev/null; \
echo '--- end arbor map ---'; exit 0"
        .to_string();
    (init, block, map)
}

/// Insert Arbor hooks into the settings tree. Returns true if anything changed.
fn ensure_hooks(settings: &mut Value) -> bool {
    let (init, block, map) = arbor_hook_commands();

    let hooks = settings
        .as_object_mut()
        .unwrap()
        .entry("hooks")
        .or_insert_with(|| json!({}));
    if !hooks.is_object() {
        *hooks = json!({});
    }
    let hooks = hooks.as_object_mut().unwrap();

    let pre = add_bash_hooks(hooks, "PreToolUse", &[init, block]);
    let post = add_bash_hooks(hooks, "PostToolUse", &[map]);
    pre || post
}

/// Ensure each command in `cmds` is registered under `event` for the Bash
/// matcher. Skips commands already present (exact match). Returns true if any
/// command was added.
fn add_bash_hooks(
    hooks: &mut serde_json::Map<String, Value>,
    event: &str,
    cmds: &[String],
) -> bool {
    let entries = hooks.entry(event.to_string()).or_insert_with(|| json!([]));
    if !entries.is_array() {
        *entries = json!([]);
    }
    let entries = entries.as_array_mut().unwrap();

    // Which commands are already installed for this event?
    let existing: Vec<String> = entries
        .iter()
        .flat_map(|group| group.get("hooks").and_then(|h| h.as_array()))
        .flatten()
        .filter_map(|h| h.get("command").and_then(|c| c.as_str()))
        .map(String::from)
        .collect();

    let to_add: Vec<&String> = cmds
        .iter()
        .filter(|c| !existing.iter().any(|e| e == *c))
        .collect();

    if to_add.is_empty() {
        return false;
    }

    // Find (or create) the Bash matcher group and append to its hooks array.
    let group = match entries
        .iter_mut()
        .find(|g| g.get("matcher").and_then(|m| m.as_str()) == Some("Bash"))
    {
        Some(g) => g,
        None => {
            entries.push(json!({ "matcher": "Bash", "hooks": [] }));
            entries.last_mut().unwrap()
        }
    };

    let group_hooks = group.as_object_mut().and_then(|o| {
        o.entry("hooks".to_string())
            .or_insert_with(|| json!([]))
            .as_array_mut()
    });
    let Some(group_hooks) = group_hooks else {
        return false;
    };

    for cmd in to_add {
        group_hooks.push(json!({ "type": "command", "command": cmd }));
    }
    true
}

/// Add the arbor command allow-list under `permissions.allow`. Returns true if
/// any entry was added.
fn ensure_permissions(settings: &mut Value) -> bool {
    let perms = settings
        .as_object_mut()
        .unwrap()
        .entry("permissions")
        .or_insert_with(|| json!({}));
    if !perms.is_object() {
        *perms = json!({});
    }
    let allow = perms
        .as_object_mut()
        .unwrap()
        .entry("allow")
        .or_insert_with(|| json!([]));
    if !allow.is_array() {
        *allow = json!([]);
    }
    let allow = allow.as_array_mut().unwrap();

    let mut changed = false;
    for entry in PERMISSIONS {
        let present = allow.iter().any(|v| v.as_str() == Some(*entry));
        if !present {
            allow.push(json!(entry));
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The regression that matters most here: this hook fires on the agent's
    /// first Bash call of the day. Without the opt-out, a JVM project with a
    /// stale SCIP index would compile — for minutes — inside the hook, with
    /// `2>/dev/null` hiding every clue as to why nothing was happening.
    #[test]
    fn the_map_hook_never_triggers_an_auto_rebuild() {
        let (_, _, map) = arbor_hook_commands();
        assert!(
            map.contains("ARBOR_NO_AUTO_REBUILD=1 arbor map"),
            "map hook must disable auto-rebuild: {map}"
        );
    }

    #[test]
    fn the_init_hook_only_initialises_and_never_indexes() {
        // `arbor init` just creates .arbor/; `arbor index` would be refused on
        // a SCIP project and would downgrade the graph elsewhere.
        let (init, _, _) = arbor_hook_commands();
        assert!(init.contains("arbor init ."));
        assert!(
            !init.contains("arbor index"),
            "the init hook must not index: {init}"
        );
    }

    #[test]
    fn no_injected_hook_can_rebuild_the_graph() {
        let (init, block, map) = arbor_hook_commands();
        for (name, cmd) in [("init", &init), ("block", &block), ("map", &map)] {
            assert!(
                !cmd.contains("arbor scip"),
                "{name} hook must not start a rebuild: {cmd}"
            );
            assert!(
                !cmd.contains("arbor index"),
                "{name} hook must not re-index: {cmd}"
            );
        }
    }

    #[test]
    fn permissions_allow_polling_a_rebuild_but_not_starting_one() {
        assert!(PERMISSIONS.iter().any(|p| p.contains("--task-status")));
        assert!(
            !PERMISSIONS.contains(&"Bash(arbor scip *)"),
            "a blanket scip allow-list would let an agent start Gradle builds unprompted"
        );
        assert!(
            !PERMISSIONS.iter().any(|p| p.contains("arbor index")),
            "indexing is never agent-initiated"
        );
    }

    #[test]
    fn permissions_are_read_only_commands_not_a_blanket_wildcard() {
        // `Bash(arbor *)` would allow `arbor scip --background` (a multi-minute
        // Gradle build) and `arbor index --force` (replaces compiler-resolved
        // edges with guesses). Arbor's own docs recommended the wildcard until
        // this was caught.
        assert!(
            !PERMISSIONS.contains(&"Bash(arbor *)"),
            "a blanket allow-list would permit rebuilds and downgrades"
        );
        for p in PERMISSIONS {
            assert!(
                !p.contains("--force"),
                "never allow-list the Tree-sitter downgrade: {p}"
            );
        }
    }
    #[test]
    fn permissions_are_all_well_formed_bash_patterns() {
        for p in PERMISSIONS {
            assert!(p.starts_with("Bash(arbor "), "malformed: {p}");
            assert!(p.ends_with(')'), "malformed: {p}");
        }
    }

    #[test]
    fn guidance_tells_the_agent_not_to_index_a_scip_project() {
        let block = directives_block();
        assert!(block.contains("SCIP index"), "guidance must cover SCIP");
        assert!(block.contains("Never run `arbor index`"));
        assert!(
            block.contains("arbor scip --background"),
            "guidance must name the refresh command for the human"
        );
        assert!(
            block.contains("--task-status"),
            "guidance must say how to poll a running rebuild"
        );
    }

    #[test]
    fn guidance_block_is_wrapped_in_the_idempotency_markers() {
        let block = directives_block();
        assert!(block.contains(BEGIN_MARKER));
        assert!(block.contains(END_MARKER));
        // Re-running `arbor hook claude` must replace, not append.
        let twice = upsert_block(&block, &block);
        assert_eq!(twice.matches(BEGIN_MARKER).count(), 1);
    }
}
