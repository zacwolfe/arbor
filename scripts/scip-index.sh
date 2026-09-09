#!/usr/bin/env bash
#
# scip-index.sh — generate a SCIP index and ingest it into Arbor.
#
# scip-java's Gradle plugin is not configuration-cache safe: it calls
# Task.project at execution time, so on any project with
# `org.gradle.configuration-cache=true` the index build fails with a
# misleading `Cannot get property 'dependenciesOut'` error. The workaround is
# to re-run with --no-configuration-cache, but args after `--` REPLACE the
# build tool's task list rather than append to it, so every task has to be
# named — and those task names are injected at runtime by scip-java's init
# script, so they do not appear in `./gradlew tasks`.
#
# This script does that for you: run scip-java once, read the task list off
# the build command it echoes, and retry with the right flag only if the first
# attempt actually failed.
#
# Usage: scripts/scip-index.sh [--no-ingest] [--dry-run] [--background]
#                              [--root <path>] [-- <arbor scip flags>]

set -euo pipefail

# Resolved before any cd, so delegation still works with a relative --root.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INGEST_SCRIPT="$SCRIPT_DIR/arbor-scip-ingest.sh"

NO_INGEST="false"
DRY_RUN="false"
BACKGROUND="false"
PROJECT_ROOT="."
EXTRA_ARBOR_ARGS=""
FORWARD=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --no-ingest)
      NO_INGEST="true"
      FORWARD+=("$1")
      shift
      ;;
    --dry-run)
      DRY_RUN="true"
      FORWARD+=("$1")
      shift
      ;;
    --background)
      BACKGROUND="true"
      shift
      ;;
    --root)
      PROJECT_ROOT="$2"
      FORWARD+=("$1" "$2")
      shift 2
      ;;
    --)
      shift
      EXTRA_ARBOR_ARGS="$*"
      FORWARD+=("--" "$@")
      break
      ;;
    -h|--help)
      sed -n '3,19p' "$0" | sed 's/^# \{0,1\}//'
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

run() {
  echo "+ $*"
  if [[ "$DRY_RUN" == "true" ]]; then
    return 0
  fi
  "$@"
}

# --- background re-exec ----------------------------------------------------
#
# A full scip-java run is a Gradle compile — minutes, not seconds. Detach it so
# the shell comes back, and keep the whole log rather than a summary, since a
# failed compile is the thing you will actually need to read.

if [[ "$BACKGROUND" == "true" ]]; then
  LOGFILE="${TMPDIR:-/tmp}/arbor-scip-$(date +%Y%m%d-%H%M%S).log"
  nohup "$SCRIPT_DIR/$(basename "$0")" ${FORWARD[@]+"${FORWARD[@]}"} \
    >"$LOGFILE" 2>&1 &
  echo "Started in background: pid $!"
  echo "  log:    $LOGFILE"
  echo "  follow: tail -f $LOGFILE"
  echo
  echo "The existing graph stays queryable until the run succeeds; a failed"
  echo "build leaves it untouched."
  exit 0
fi

# --- preflight -------------------------------------------------------------

command -v scip-java >/dev/null 2>&1 \
  || die "scip-java not on PATH. See the JVM section of Arbor's README."

cd "$PROJECT_ROOT" || die "cannot cd to $PROJECT_ROOT"

if [[ ! -f build.gradle && ! -f build.gradle.kts && ! -f pom.xml && ! -f build.sbt ]]; then
  die "no build.gradle/.kts, pom.xml, or build.sbt here. Run this from the project root."
fi

if [[ "$NO_INGEST" == "false" ]] && ! command -v arbor >/dev/null 2>&1; then
  die "arbor not on PATH. Install it, or pass --no-ingest."
fi

LOG="$(mktemp -t scip-index)"
trap 'rm -f "$LOG"' EXIT

# --- attempt 1: let scip-java try on its own -------------------------------
#
# Deliberately not skipped in favour of going straight to the workaround: on a
# project without the configuration cache this succeeds, and a second full
# build would be pure waste.

echo "==> scip-java index (attempt 1)"
rc=0
if [[ "$DRY_RUN" == "true" ]]; then
  echo "+ scip-java index"
else
  scip-java index 2>&1 | tee "$LOG" || rc=$?
fi

if [[ "$rc" -ne 0 ]]; then
  echo
  echo "==> attempt 1 failed (exit $rc); inspecting output"

  # scip-java echoes the build command it ran, prefixed with '$'. The task
  # list is the trailing run of tokens that are neither options nor paths.
  BUILD_LINE="$(grep -m1 '^\$ ' "$LOG" | sed 's/^\$ //' || true)"
  [[ -n "$BUILD_LINE" ]] \
    || die "could not find scip-java's build command in its output; nothing to reconstruct.
Last 20 lines:
$(tail -20 "$LOG")"

  read -r -a TOKENS <<< "$BUILD_LINE"

  TASKS=""
  i=$(( ${#TOKENS[@]} - 1 ))
  while [[ $i -ge 0 ]]; do
    tok="${TOKENS[$i]}"
    case "$tok" in
      -*) break ;;   # an option: task list has ended
      */*) break ;;  # a path (e.g. --init-script's argument), not a task
    esac
    TASKS="$tok${TASKS:+ $TASKS}"
    i=$(( i - 1 ))
  done

  [[ -n "$TASKS" ]] \
    || die "found scip-java's build command but no task list in it:
  $BUILD_LINE"

  echo "    build tool: ${TOKENS[0]##*/}"
  echo "    task list:  $TASKS"

  # Only add the flag when the failure was actually the configuration cache.
  # Adding it blindly would mask an unrelated build break as a scip problem.
  EXTRA=""
  if grep -qi "configuration cache\|configuration-cache" "$LOG"; then
    case "${TOKENS[0]}" in
      *gradlew|*gradle)
        EXTRA="--no-configuration-cache"
        echo "    detected: Gradle configuration cache (scip-java's plugin is not cache-safe)"
        ;;
    esac
  fi

  if [[ -z "$EXTRA" ]]; then
    echo
    echo "This failure is not the Gradle configuration cache, so there is no" >&2
    echo "flag to add — the build itself needs fixing. Last 30 lines:" >&2
    echo >&2
    tail -30 "$LOG" >&2
    exit "$rc"
  fi

  echo
  echo "==> scip-java index (attempt 2, with $EXTRA)"
  # shellcheck disable=SC2086  # TASKS and EXTRA are intentionally word-split
  run scip-java index -- $TASKS $EXTRA
fi

# --- verify and ingest ----------------------------------------------------
#
# Delegated so the index-collection rules (skip empties, one arbor call for
# every module) live in exactly one place rather than drifting between the two
# scripts.

if [[ "$DRY_RUN" == "true" ]]; then
  echo
  echo "dry run: stopping before verification and ingest."
  exit 0
fi

[[ -x "$INGEST_SCRIPT" ]] || die "missing or non-executable: $INGEST_SCRIPT"

echo
if [[ "$NO_INGEST" == "true" ]]; then
  exec "$INGEST_SCRIPT" --root . --list
fi

"$INGEST_SCRIPT" --root . ${EXTRA_ARBOR_ARGS:+-- $EXTRA_ARBOR_ARGS}

echo
echo "Done. A partial build yields a partial graph — if the definition count"
echo "looks low for this repo, check for compile or codegen failures above."
