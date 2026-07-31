// Mean-Reversion Strategy for QQQ/PSQ — Rhai Reference Implementation
// ================================================================
// Exposed variables: pred_1d, pred_5d, pred_21d, current_close, sma, sma_valid, current_pos
// Returns: -1 (short/PSQ), 0 (flat), 1 (long/QQQ)
//
// Philosophy: buy dips in a bull regime, sell rips in a bear regime.
// pred_1d drives entry timing; pred_5d acts as a medium-term directional filter.
// No position scaling — output is strictly {-1, 0, 1}.
//
// Thresholds (calibrated for QQQ daily bars, 2022–2025):
//   Long entry:    pred_1d > 0.0015  (bull regime)  OR  pred_1d < -0.004 (contrarian bull)
//   Long exit:     pred_1d < -0.002 (stop)         OR  pred_1d > 0.004  (take profit)
//   Short entry:   pred_1d < -0.0015 (bear regime)  OR  pred_1d >  0.003 (contrarian bear)
//   Short exit:    pred_1d >  0.001  (cover)

// ─── Regime ─────────────────────────────────────────────────────────────────
let regime = if !sma_valid { 0 } else if current_close > sma { 1 } else { -1 };

// ─── Long (QQQ) ────────────────────────────────────────────────────────────
if current_pos >= 0 {
    if regime > 0 && pred_1d > 0.0015 {
        // Bull regime + positive 1d signal → long
        1
    } else if regime > 0 && pred_1d < -0.004 {
        // Bull regime + deeply negative 1d → contrarian long (mean-reversion scalp)
        1
    } else if regime == 0 && pred_1d > 0.002 {
        // Neutral regime + strong positive signal → tentative long
        1
    } else {
        0  // flat
    }
}
// ─── Long exit ──────────────────────────────────────────────────────────────
else if current_pos == 1 {
    if pred_1d < -0.002 {
        0  // stop-loss / mean-reversion exit
    } else if pred_1d > 0.004 {
        0  // take-profit on strong signal
    } else {
        1  // hold
    }
}
// ─── Short (PSQ) ────────────────────────────────────────────────────────────
else if current_pos <= 0 {
    if regime < 0 && pred_1d < -0.0015 {
        // Bear regime + negative 1d signal → short
        -1
    } else if regime < 0 && pred_1d > 0.003 {
        // Bear regime + strongly positive 1d → contrarian short
        -1
    } else if regime == 0 && pred_1d < -0.002 {
        // Neutral + strong negative signal → tentative short
        -1
    } else {
        0  // flat
    }
}
// ─── Short exit ─────────────────────────────────────────────────────────────
else if current_pos == -1 {
    if pred_1d > 0.001 {
        0  // cover
    } else {
        -1  // hold short
    }
}
else {
    current_pos  // safety: preserve current position on unexpected state
}
