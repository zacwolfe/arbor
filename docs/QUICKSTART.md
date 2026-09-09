# Arbor Quickstart

Get AI-ready code context in 5 minutes.

> Updated for the March 2026 CLI surface.

## Install

For reproducible environments (CI/team onboarding), prefer pinning a release tag in your install command.

**No-build install (recommended):**

```bash
curl -fsSL https://raw.githubusercontent.com/Anandb71/arbor/main/scripts/install.sh | bash
```

**Windows PowerShell:**

```powershell
irm https://raw.githubusercontent.com/Anandb71/arbor/main/scripts/install.ps1 | iex
```

**Cargo alternative:**

```bash
cargo install arbor-graph-cli
```

> **Note:** The GUI is included in the binary. Run `arbor gui` after installation.
> Need version pinning/manual assets? See [INSTALL.md](./INSTALL.md).

## Initialize (Optional)

```bash
cd your-project
arbor init
```

This creates `.arbor/` with default configuration.

> Prefer a one-shot start? Use `arbor setup` to initialize and index in a single command.

## Index

```bash
arbor index
```

Parses your codebase and builds a relationship graph. Subsequent runs use caching for faster updates.

```bash
# Fast refresh during active refactors
arbor index --changed-only
```

> If `.arbor/` doesn't exist, Arbor now auto-creates it on first index/query/refactor/explain.

## Query

```bash
# Show project stats
arbor status

# List all indexed files
arbor status --files

# Search for a symbol
arbor query parse_file

# Search in a different path
arbor query parse_file ../another-project

# Get refactoring context
arbor refactor UserService

# Explain a function's dependencies
arbor explain validate_input

# Preview impact for current git diff
arbor diff

# CI safety gate (fails on risky blast radius)
arbor check --max-blast-radius 30

# Machine-readable output for CI bots
arbor check --json --max-blast-radius 30

# Jump directly to a symbol in your editor
arbor open parse_file
```

## Navigate the graph

```bash
arbor map . --exclude-test          # ranked project skeleton — start here
arbor callers "symbol" .            # who calls this?
arbor callees "symbol" .            # what does this call?
arbor entry-points .                # HTTP handlers, main, jobs, webhooks
arbor file-graph "src/Foo.java" .   # symbols + edges in one file
arbor inspect "symbol" .            # full detail on one symbol
arbor path "start" "end" .          # shortest call-graph path
```

All of these accept `--json`.

## Wire up an AI agent

```bash
arbor hook claude          # installs directives + hooks into .claude/
arbor hook claude --global # or into your user config
```

This writes an Arbor block into `CLAUDE.md`, registers PreToolUse/PostToolUse
hooks (auto-init, grep blocking, daily project-skeleton injection), and
allow-lists the read-only arbor commands so the agent runs them without a
prompt. Re-running updates the block in place.

## Compiler-accurate graphs via SCIP

Tree-sitter cannot resolve `obj.method()` — that needs the type of `obj`. Ingest
a [SCIP](https://github.com/scip-code/scip) index instead, from whichever
indexer covers your language:

```bash
scip-java index                     # Java, Kotlin — requires JDK 17+; a full compile
# scip-typescript index             # TypeScript, JavaScript
# scip-python index .               # Python
# rust-analyzer scip .              # Rust
arbor scip index.scip --root .
```

`scip-java` is the only one Arbor runs for you (`arbor scip --background`); the
others you run yourself. See [SCIP.md](SCIP.md) for the full list.

Then query exactly as above — same commands, exact edges. Refresh after code
changes with `arbor scip --background` (detached) and poll it with
`arbor scip --task-status`.

Once a project has a SCIP graph, `arbor index` is refused so Tree-sitter cannot
silently downgrade it; `arbor index --force` is the deliberate override. Set
`ARBOR_NO_AUTO_REBUILD=1` in CI, where a read command turning into a
multi-minute build is not acceptable. Full detail: [SCIP.md](SCIP.md).

## Agent workflows

```bash
arbor agent review .    # autonomous PR architecture review
arbor agent onboard .   # contributor onboarding guide
arbor agent guard .     # architecture violation check
arbor summary .         # auto-generate a PR description
```

## Use the GUI

```bash
arbor gui
```

Opens the native graphical interface for impact analysis:
- Enter a symbol name
- Click "Analyze" to see callers, dependencies, and confidence
- File paths are hidden by default for privacy (click to reveal)
- Copy results as Markdown for PRs

![Arbor GUI](gui_screenshot.png)

## Watch Mode

Auto-refresh the index when files change:

```bash
arbor watch
```

Great for development workflows where you want continuous indexing.

## Generate PR Summaries

```bash
arbor pr-summary parse_file,validate_input
```

Generates a Markdown summary of impact for multiple changed symbols.

## Use with Cursor

1. Add to `.cursor/mcp.json`:
```json
{
  "mcpServers": {
    "arbor": {
      "command": "arbor",
      "args": ["bridge"]
    }
  }
}
```

2. Restart Cursor.

3. Ask questions like:
   - "What depends on `UserService`?"
   - "What does `parse_file` call?"
   - "Show me the context for refactoring `validate`"

## CLI Flags

| Flag | Description |
|------|-------------|
| `--no-cache` | Force full re-index (skip cache) |
| `--follow-symlinks` | Include symlinked directories |
| `--files` | Show detailed file stats in `status` |
| `--depth N` | Set impact analysis depth (default: 5) |
| `--why` | Show detailed reasoning for each affected node |
| `--json` | Output as JSON instead of formatted text |

## Health Check

```bash
arbor doctor
```

Runs environment diagnostics (ports, workspace layout, visualizer and extension presence).

## Team Workflow (Recommended)

```bash
# after pulling changes
arbor index --changed-only

# before opening a PR
arbor diff
arbor check --max-blast-radius 30
```

If `arbor check` fails, run focused tests before merge and include blast-radius notes in your PR description.

## Next Steps

- [Roadmap](./ROADMAP.md) — See what's coming
- [Architecture Guide](./ARCHITECTURE.md)
- [Supported Languages](./ADDING_LANGUAGES.md)
- [MCP Protocol](./PROTOCOL.md)
- [MCP Integration](./MCP_INTEGRATION.md)
- [Changelog](../CHANGELOG.md) — full release history
- [SCIP ingestion](SCIP.md) — compiler-accurate graphs for JVM projects
- [Glama MCP Directory Listing](https://glama.ai/mcp/servers/@Anandb71/arbor)
