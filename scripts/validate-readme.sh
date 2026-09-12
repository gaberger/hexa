#!/usr/bin/env bash
#
# validate-readme.sh — run `hexa readme validate` from the repo root.
#
# ADR-2026-04-11-0227: README claim validation.
#
# Usage:
#   ./scripts/validate-readme.sh           # advisory mode — exits 0 on warnings
#   ./scripts/validate-readme.sh --strict  # CI mode — exits 1 on warnings too
#
# Checks performed:
#   * Numeric counts (ADRs, agents, skills, WASM modules, port traits, reducers)
#   * SVG asset file existence
#   * Internal markdown link resolution
#   * Named entity references (modules, crates, agents)
#   * CLI command existence via `hexa <cmd> --help`
#
# Exit codes:
#   0 — all checks passed (or only warnings without --strict)
#   1 — at least one check failed
#   2 — hexa binary could not be built
#
# The canonical test for CI is `cargo test -p hexa-cli` which runs
# `repo_readme_is_accurate` — this script is the fast interactive equivalent.

set -euo pipefail

# Resolve repo root from this script's location
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

# Prefer an existing debug build for speed; build if nothing found.
HEXA_BIN=""
for candidate in "target/release/hexa" "target/debug/hexa"; do
    if [[ -x "${candidate}" ]]; then
        HEXA_BIN="${candidate}"
        break
    fi
done

if [[ -z "${HEXA_BIN}" ]]; then
    echo "no hexa binary found — building with cargo build -p hexa-cli..."
    if ! cargo build -p hexa-cli >&2; then
        echo "error: failed to build hexa-cli" >&2
        exit 2
    fi
    HEXA_BIN="target/debug/hexa"
fi

echo "using: ${HEXA_BIN}"
exec "${HEXA_BIN}" readme validate "$@"
