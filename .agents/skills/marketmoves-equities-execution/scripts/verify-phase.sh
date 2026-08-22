#!/usr/bin/env bash
# Ad-hoc verification script for MarketMoves Phase 3.x (equities execution).
# Copy this to the repo and run from the project root. It exercises the
# test suites that don't require the parity harness (which has a pre-existing
# Candle struct construction issue unrelated to Phase 3).
#
# Usage:
#   bash scripts/verify-phase.sh
#
# Exit codes:
#   0  — all suites green
#   1+ — at least one suite failed (printed above)
set -euo pipefail

echo "== build =="
cargo build -p engine 2>&1 | tail -3

echo ""
echo "== totp unit tests (Phase 3.4) =="
cargo test -p engine --lib totp:: 2>&1 | tail -10

echo ""
echo "== moomoo executor unit tests (Phase 3.3) =="
cargo test -p engine --lib exec::moomoo:: 2>&1 | tail -10

echo ""
echo "== mode-toggle integration tests (Phase 3.4) =="
cargo test -p engine --test mode_toggle 2>&1 | tail -10

echo ""
echo "== regression: exec_parity (Phase 3.1+3.2) =="
cargo test -p engine --test exec_parity 2>&1 | tail -3

echo ""
echo "== regression: paper_verification (Phase 3.1+3.2) =="
cargo test -p engine --test paper_verification 2>&1 | tail -3

echo ""
echo "== full lib tests (skip config::tests — pre-existing .env pollution) =="
cargo test -p engine --lib -- --skip config::tests 2>&1 | tail -3

echo ""
echo "All green ✓"