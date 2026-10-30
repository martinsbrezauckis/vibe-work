#!/usr/bin/env bash
# The one command every change must pass: formatting, lints, tests, size and public-paths guards.
#
# Role: single entry point for local runs and CI. Depends on: cargo, pnpm (node_modules installed),
# node 20+, git. Called by: developers, agents, CI. Run from anywhere: it changes to the repo root.
#
# Heavy commands run under a 16 GB memory cap when `systemd-run` exists (Linux), because an
# out-of-memory kill can take down a whole WSL VM. On systems without it they run directly.
# Each step prints "== <name>" first; a failing step is reported and the rest still run (collecting mode),
# then the script exits 1 listing every failed step (VW_CHECK_FAIL_FAST=1: stop at the first failure).
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/../.."
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}"

# Plain fixture folders must not discover a repository above the checkout.
if [ "$(git -C .. rev-parse --is-inside-work-tree 2>/dev/null || true)" = true ]; then
  vw_check_test_root=""
  for temp_base in "${TMPDIR:-/tmp}" /tmp; do
    candidate="$(mktemp -d "$temp_base/vw-tests.XXXXXXXX")" || continue
    if [ "$(git -C "$candidate" rev-parse --is-inside-work-tree 2>/dev/null || true)" != true ]; then
      vw_check_test_root="$candidate"
      break
    fi
    rmdir -- "$candidate"
  done
  if [ -z "$vw_check_test_root" ]; then
    echo "could not create a test root outside a Git work tree" >&2
    exit 1
  fi
  cleanup_test_root() {
    # Vibe's immutable package copies can contain directories without write permission.
    find "$vw_check_test_root" -type d -exec chmod u+w {} +
    rm -rf -- "$vw_check_test_root"
  }
  trap cleanup_test_root EXIT
  VW_TEST_ROOT="$(cd "$vw_check_test_root" && pwd -P)"
  if command -v cygpath >/dev/null 2>&1; then
    VW_TEST_ROOT="$(cygpath -am "$VW_TEST_ROOT")"
  fi
  export VW_TEST_ROOT
  echo "Using a temporary test root because the checkout's parent is inside a Git work tree."
fi

# Runs a command under the memory cap when available.
capped() {
  if command -v systemd-run >/dev/null 2>&1 && systemd-run --user --scope -q true >/dev/null 2>&1; then
    systemd-run --user --scope -q -p "MemoryMax=${VW_CHECK_MEMORY_MAX:-16G}" -p MemorySwapMax=0 -- "$@"
  else
    "$@"
  fi
}

# step "<name>" <command...>: announce, run, and record a failure with a clear message.
# Collecting mode:
# a failed step is recorded and the next one runs; the script fails at the end listing every failed step.
# VW_CHECK_FAIL_FAST=1 stops at the first failure instead (quick local iterations).
failed_steps=()
step() {
  local name="$1"
  shift
  echo "== ${name}"
  if ! "$@"; then
    echo "check.sh FAILED at step: ${name}" >&2
    failed_steps+=("${name}")
    if [ "${VW_CHECK_FAIL_FAST:-0}" = 1 ]; then
      exit 1
    fi
  fi
}

# Regenerates the protocol types in a scratch directory and fails on any difference from the
# committed files. Needs a Python with the pinned mistral-vibe installed: $VIBE_PYTHON, else the
# repo's versioned runtime (linked worktrees find the main checkout's data/ through the git common dir).
generated_code_check() {
  local python="${VIBE_PYTHON:-}" main pin
  pin="$(cat tools/public/codegen/vibe.pin)"
  if [ -z "$python" ]; then
    main="$(cd "$(git rev-parse --git-common-dir)/.." && pwd)"
    for candidate in "$PWD/data/vibe-$pin/venv/bin/python" "$main/data/vibe-$pin/venv/bin/python"; do
      if [ -x "$candidate" ]; then python="$candidate"; break; fi
    done
  fi
  if [ -z "$python" ]; then
    echo "no Python with mistral-vibe found: set VIBE_PYTHON (see tools/public/codegen/vibe.pin)" >&2
    return 1
  fi
  "$python" "$1" --check
}

# A fresh checkout or worktree has no node_modules; say so instead of failing deep inside tsc.
deps_installed() {
  if [ ! -d ui/node_modules ]; then
    echo "ui/node_modules is missing: run 'pnpm install --frozen-lockfile' at the repo root" >&2
    return 1
  fi
}

started=$SECONDS
step "deps installed" deps_installed
step "rust: fmt" cargo fmt --all --check
# Release builds of the Tauri crate embed ui/dist, so the bundle is built first; this also
# proves the UI compiles before the (slower) Rust steps run.
step "ui: build (typecheck + bundle)" capped pnpm --filter vibe-work-ui build
step "ui: first-paint check" node scripts/public/check_first_paint.mjs
step "rust: clippy" capped cargo clippy --workspace --all-targets -- -D warnings
# The TypeScript generator is behind the `ts` feature, which the workspace build leaves off.
step "rust: clippy (core TS generator)" capped cargo clippy -p vw-core-api --all-targets --features ts -- -D warnings
step "rust: test" capped cargo test --workspace --no-fail-fast
step "ui: typecheck" pnpm --filter vibe-work-ui typecheck
step "ui: lint" pnpm --filter vibe-work-ui lint
step "ui: format" pnpm --filter vibe-work-ui format:check
step "ui: test" capped pnpm --filter vibe-work-ui test
step "size check" node scripts/public/check_size.mjs
step "colour check (tokens only)" node scripts/public/check_colors.mjs
step "public-paths scan" node scripts/public/check_public_paths.mjs
step "generated-code check" generated_code_check tools/public/codegen/generate.py
step "skills generated-code check" generated_code_check crates/vw-skills/tools/generate.py
step "vocabulary reference drift" node tools/public/codegen/vocabulary.mjs --check
# ui/src/protocol/core/ is generated from the Rust core API types (crates/vw-core-api, ts-rs).
step "core API TypeScript drift" capped cargo run -q -p vw-core-api --features ts --bin gen-core-ts -- --check

if [ "${#failed_steps[@]}" -gt 0 ]; then
  echo "check.sh FAILED: ${#failed_steps[@]} step(s) after $((SECONDS - started))s:" >&2
  printf '  - %s\n' "${failed_steps[@]}" >&2
  exit 1
fi
echo "check.sh PASSED in $((SECONDS - started))s"
