#!/usr/bin/env bash
#
# arbor-scip-ingest.sh — find every SCIP index in a project and ingest them.
#
# Replaces the naive form:
#
#     arbor scip $(find . -name 'index.scip') --root .
#
# which has three problems: it word-splits on paths containing spaces, it
# silently passes nothing (indexing nothing) when no index exists, and it
# happily feeds arbor zero-byte indexes from a failed build.
#
# Every index must go in ONE arbor call. A symbol defined in module B is only
# linkable while B's definitions are in scope, so ingesting modules one at a
# time leaves every cross-module edge unresolved.
#
# Usage: arbor-scip-ingest.sh [--root <path>] [--list] [--dry-run] [-- <arbor flags>]
#
#   --root <path>   Project root to search and pass to arbor (default: .)
#   --list          Print the indexes that would be ingested, then stop
#   --dry-run       Print the arbor command without running it
#   -- <flags>      Passed through to `arbor scip` (e.g. --merge, --no-dispatch)

set -euo pipefail

PROJECT_ROOT="."
LIST_ONLY="false"
DRY_RUN="false"
EXTRA_ARGS=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --root)
      PROJECT_ROOT="$2"
      shift 2
      ;;
    --list)
      LIST_ONLY="true"
      shift
      ;;
    --dry-run)
      DRY_RUN="true"
      shift
      ;;
    --)
      shift
      while [[ $# -gt 0 ]]; do
        EXTRA_ARGS+=("$1")
        shift
      done
      ;;
    -h|--help)
      sed -n '3,22p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "error: unknown option '$1' (try --help)" >&2
      exit 2
      ;;
  esac
done

die() {
  echo "error: $*" >&2
  exit 1
}

cd "$PROJECT_ROOT" || die "cannot cd to $PROJECT_ROOT"

if [[ "$LIST_ONLY" == "false" ]] && ! command -v arbor >/dev/null 2>&1; then
  die "arbor not on PATH. Install it, or use --list."
fi

# --- collect ---------------------------------------------------------------
#
# `-size +0` skips the empty indexes a failed build leaves behind — arbor
# rejects them anyway, but with an error that reads as an arbor problem rather
# than a build one. Read via a loop rather than command substitution so paths
# containing spaces survive.

INDEXES=()
EMPTY_COUNT=0

while IFS= read -r found; do
  INDEXES+=("$found")
done < <(find . -name 'index.scip' -type f -size +0 \
           -not -path './.git/*' -not -path './.arbor/*' 2>/dev/null | sort)

while IFS= read -r _; do
  EMPTY_COUNT=$(( EMPTY_COUNT + 1 ))
done < <(find . -name 'index.scip' -type f -empty \
           -not -path './.git/*' -not -path './.arbor/*' 2>/dev/null)

if [[ "$EMPTY_COUNT" -gt 0 ]]; then
  echo "warning: ignoring $EMPTY_COUNT empty index.scip file(s) — a build that" >&2
  echo "         produced them did not complete." >&2
fi

if [[ "${#INDEXES[@]}" -eq 0 ]]; then
  die "no non-empty index.scip found under $(pwd).
Generate one first:  scripts/scip-index.sh"
fi

echo "==> ${#INDEXES[@]} index file(s):"
for f in "${INDEXES[@]}"; do
  echo "    $f ($(wc -c <"$f" | tr -d ' ') bytes)"
done

if [[ "${#INDEXES[@]}" -eq 1 ]]; then
  :
else
  echo "    (all passed in one call so cross-module edges resolve)"
fi

# --- ingest ----------------------------------------------------------------

CMD=(arbor scip "${INDEXES[@]}" --root .)
if [[ "${#EXTRA_ARGS[@]}" -gt 0 ]]; then
  CMD+=("${EXTRA_ARGS[@]}")
fi

if [[ "$LIST_ONLY" == "true" ]]; then
  echo
  echo "To ingest:"
  printf '  '
  printf '%q ' "${CMD[@]}"
  echo
  exit 0
fi

echo
echo "+ ${CMD[*]}"
if [[ "$DRY_RUN" == "true" ]]; then
  exit 0
fi

"${CMD[@]}"
