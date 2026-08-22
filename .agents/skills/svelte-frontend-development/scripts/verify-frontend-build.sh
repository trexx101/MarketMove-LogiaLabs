#!/usr/bin/env bash
# Template: Build-based verification for Svelte 4 frontends with no test suite.
# Copy to /tmp/hermes-verify-frontend.sh, adapt EXPECTED_FILES / stores / events,
# then run: bash /tmp/hermes-verify-frontend.sh
# Clean up: rm /tmp/hermes-verify-frontend.sh
set -euo pipefail

# ── Adapt these for your project ──────────────────────────
PROJECT="/abs/path/to/frontend"
EXPECTED_FILES=(
  "src/lib/stores.js"
  "src/lib/websocket.js"
  "src/lib/api.js"
  "src/lib/components/StatusPanel.svelte"
  "src/lib/components/CandlestickChart.svelte"
  "src/views/Dashboard.svelte"
  "src/App.svelte"
  # ... add all your source files
)
EXPECTED_STORES="wsConnected status predictions features trades accuracy chartData"
EXPECTED_WS_EVENTS="PnlTick PredictionUpdate FeatureUpdate TradeFill ModeChange StalenessAlert"
EXPECTED_API_FUNCS="fetchStatus fetchPredictions fetchChart fetchAccuracy"
EXPECTED_ROUTES="dashboard strategy ledger advisor"
# ──────────────────────────────────────────────────────────

FAIL=0

echo "=== Frontend Verification ==="
echo ""

# 1. File existence
echo "--- [1] File existence check ---"
for f in "${EXPECTED_FILES[@]}"; do
  if [ -f "$PROJECT/$f" ]; then
    echo "  OK   $f"
  else
    echo "  FAIL $f (missing)"
    FAIL=1
  fi
done
echo ""

# 2. Store exports
echo "--- [2] Store exports check ---"
for store in $EXPECTED_STORES; do
  if grep -q "export const $store" "$PROJECT/src/lib/stores.js"; then
    echo "  OK   store: $store"
  else
    echo "  FAIL store: $store not exported"
    FAIL=1
  fi
done
echo ""

# 3. WebSocket event handlers
echo "--- [3] WebSocket manager check ---"
for fn in connectWebSocket disconnectWebSocket; do
  if grep -q "export function $fn" "$PROJECT/src/lib/websocket.js"; then
    echo "  OK   function: $fn"
  else
    echo "  FAIL function: $fn missing"
    FAIL=1
  fi
done
for evt in $EXPECTED_WS_EVENTS; do
  if grep -q "'$evt'" "$PROJECT/src/lib/websocket.js"; then
    echo "  OK   event handler: $evt"
  else
    echo "  FAIL event handler: $evt missing"
    FAIL=1
  fi
done
echo ""

# 4. API functions
echo "--- [4] API functions check ---"
for fn in $EXPECTED_API_FUNCS; do
  if grep -q "export async function $fn" "$PROJECT/src/lib/api.js"; then
    echo "  OK   $fn"
  else
    echo "  FAIL $fn missing"
    FAIL=1
  fi
done
echo ""

# 5. No Svelte 5 runes
echo "--- [5] Svelte 4 syntax check ---"
RUNE_FILES=$(grep -rl '\$state\|\$derived\|\$effect' "$PROJECT/src" --include='*.svelte' --include='*.js' 2>/dev/null || true)
if [ -z "$RUNE_FILES" ]; then
  echo "  OK   no Svelte 5 runes detected"
else
  echo "  FAIL Svelte 5 runes found in: $RUNE_FILES"
  FAIL=1
fi
echo ""

# 6. No external chart libs (optional — remove if you use them)
echo "--- [6] No external chart libs check ---"
CHART_LIBS=$(grep -rl "d3\|chart\.js\|uplot\|echarts\|plotly" "$PROJECT/src" --include='*.svelte' --include='*.js' 2>/dev/null || true)
if [ -z "$CHART_LIBS" ]; then
  echo "  OK   no external chart libraries"
else
  echo "  FAIL external chart lib found in: $CHART_LIBS"
  FAIL=1
fi
echo ""

# 7. Canvas usage in chart components
echo "--- [7] Canvas-based charts check ---"
for comp in "$PROJECT"/src/lib/components/*Chart*.svelte "$PROJECT"/src/lib/components/*Curve*.svelte; do
  [ -f "$comp" ] || continue
  if grep -q "canvas" "$comp"; then
    echo "  OK   $(basename $comp) uses canvas"
  else
    echo "  FAIL $(basename $comp) does not use canvas"
    FAIL=1
  fi
done
echo ""

# 8. Production build
echo "--- [8] Production build (npm run build) ---"
BUILD_OUTPUT=$(cd "$PROJECT" && npm run build 2>&1)
BUILD_EXIT=$?
if [ $BUILD_EXIT -eq 0 ]; then
  echo "  OK   build exited 0"
  if echo "$BUILD_OUTPUT" | grep -qi "warning"; then
    echo "  WARN  build output contains warnings"
  else
    echo "  OK   no warnings in build output"
  fi
  if [ -f "$PROJECT/dist/index.html" ]; then
    echo "  OK   dist/index.html generated"
  else
    echo "  FAIL dist/index.html not found"
    FAIL=1
  fi
else
  echo "  FAIL build exited with code $BUILD_EXIT"
  echo "$BUILD_OUTPUT" | tail -20
  FAIL=1
fi
echo ""

# 9. WS lifecycle in views
echo "--- [9] Dashboard lifecycle check ---"
for view in "$PROJECT"/src/views/*.svelte; do
  [ -f "$view" ] || continue
  if grep -q "connectWebSocket" "$view" && grep -q "disconnectWebSocket" "$view"; then
    echo "  OK   $(basename $view) connects/disconnects WS"
  fi
done
echo ""

# 10. Routing in App.svelte
echo "--- [10] App.svelte routing check ---"
for view in $EXPECTED_ROUTES; do
  if grep -q "currentView === '$view'" "$PROJECT/src/App.svelte"; then
    echo "  OK   route: $view"
  else
    echo "  FAIL route: $view missing"
    FAIL=1
  fi
done
echo ""

# Summary
echo "=== Summary ==="
if [ $FAIL -eq 0 ]; then
  echo "ALL CHECKS PASSED"
  exit 0
else
  echo "SOME CHECKS FAILED"
  exit 1
fi
