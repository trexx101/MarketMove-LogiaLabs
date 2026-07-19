use crate::db::Candle;

/// Per-candle features that mirror the Colab training pipeline.
///
/// Feature contract (must stay byte-identical to the Colab `SwingTradingDataset`):
///   - `log_return` = ln(close[t] / close[t-1]); 0.0 for the first candle.
///   - `atr_72`     = simple rolling mean of True Range, window = 72.
///   - `vwap_dev`   = ln(close / rolling_vwap) where `rolling_vwap` is the
///                    Colab-style rolling VWAP = Σ(typical_price × volume) /
///                    Σ(volume) over the same 72-window. We do NOT use the
///                    exchange-reported `vwap` field — that diverges from the
///                    training definition and silently shifts model inputs.
#[derive(Debug, Clone, PartialEq)]
pub struct FeatureRow {
    /// ln(close[t] / close[t-1]); 0.0 for the first candle.
    pub log_return: f64,
    /// Rolling mean of True Range, window = 72, min_periods = 1
    /// (pandas-style simple rolling mean, **not** Wilder's EMA).
    pub atr_72: f64,
    /// ln(close / rolling_vwap); 0.0 when rolling_vwap <= 0.0.
    pub vwap_dev: f64,
}

/// Compute features for every candle in `candles`.
///
/// Returns a `Vec<FeatureRow>` with the same length as the input slice.
pub fn compute_features(candles: &[Candle]) -> Vec<FeatureRow> {
    let n = candles.len();
    if n == 0 {
        return Vec::new();
    }

    // --- pre-compute True Range for each candle ---------------------------
    let mut tr = Vec::with_capacity(n);
    for (i, c) in candles.iter().enumerate() {
        let t = if i == 0 {
            c.high - c.low
        } else {
            let prev_close = candles[i - 1].close;
            let hl = c.high - c.low;
            let hpc = (c.high - prev_close).abs();
            let lpc = (c.low - prev_close).abs();
            hl.max(hpc).max(lpc)
        };
        tr.push(t);
    }

    // --- build final FeatureRows ------------------------------------------
    const WINDOW: usize = 72;
    let mut rows = Vec::with_capacity(n);

    for i in 0..n {
        let c = &candles[i];

        // log return
        let log_return = if i == 0 {
            0.0
        } else {
            let prev = candles[i - 1].close;
            if prev > 0.0 {
                (c.close / prev).ln()
            } else {
                0.0
            }
        };

        // ATR: rolling mean of TR, window=72, min_periods=1
        let start = i.saturating_sub(WINDOW - 1);
        let slice = &tr[start..=i];
        let atr_72 = slice.iter().sum::<f64>() / slice.len() as f64;

        // Rolling VWAP (Colab definition): Σ(typical_price × volume) /
        // Σ(volume) over the same 72-window. typical_price = (H + L + C) / 3.
        // We deliberately do NOT use the exchange-reported `vwap` field.
        let vstart = i.saturating_sub(WINDOW - 1);
        let mut num = 0.0_f64;
        let mut den = 0.0_f64;
        for j in vstart..=i {
            let cj = &candles[j];
            let tp = (cj.high + cj.low + cj.close) / 3.0;
            num += tp * cj.volume;
            den += cj.volume;
        }
        let rolling_vwap = if den > 0.0 { num / den } else { 0.0 };

        // vwap_dev = ln(close / rolling_vwap) — matches Colab np.log(close / vwap).
        let vwap_dev = if rolling_vwap > 0.0 && c.close > 0.0 {
            (c.close / rolling_vwap).ln()
        } else {
            0.0
        };

        rows.push(FeatureRow {
            log_return,
            atr_72,
            vwap_dev,
        });
    }

    rows
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    fn candle(ts: i64, open: f64, high: f64, low: f64, close: f64, volume: f64, vwap: f64) -> Candle {
        Candle { ts, open, high, low, close, volume, vwap }
    }

    #[test]
    fn compute_features_empty_returns_empty() {
        let result = compute_features(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn compute_features_single_candle() {
        let c = candle(0, 100.0, 110.0, 90.0, 105.0, 1000.0, 102.0);
        let rows = compute_features(&[c]);
        assert_eq!(rows.len(), 1);
        let row = &rows[0];

        assert_eq!(row.log_return, 0.0, "first candle log_return must be 0");

        let expected_tr = 110.0 - 90.0; // high - low for first candle
        assert!(
            (row.atr_72 - expected_tr).abs() < 1e-12,
            "atr_72 should equal TR={expected_tr}, got {}",
            row.atr_72
        );

        // vwap_dev = ln(close / rolling_vwap). For a single candle the rolling
        // VWAP equals typical_price = (H+L+C)/3.
        let tp = (110.0 + 90.0 + 105.0) / 3.0;
        let expected_vwap_dev = (105.0_f64 / tp).ln();
        assert!(
            (row.vwap_dev - expected_vwap_dev).abs() < 1e-12,
            "vwap_dev mismatch: expected {expected_vwap_dev}, got {}",
            row.vwap_dev
        );
    }

    #[test]
    fn compute_features_vwap_dev_is_log_of_close_over_rolling_vwap() {
        // Two candles; verify rolling VWAP over the 72-window and the log form.
        let c0 = candle(0, 100.0, 110.0, 90.0, 105.0, 1000.0, 102.0);
        let c1 = candle(3600, 105.0, 115.0, 95.0, 110.0, 2000.0, 108.0);
        let rows = compute_features(&[c0, c1]);
        assert_eq!(rows.len(), 2);

        // At index 1: rolling VWAP = Σ(tp*vol)/Σ(vol) over c0,c1.
        let tp0 = (110.0 + 90.0 + 105.0) / 3.0; // 101.666...
        let tp1 = (115.0 + 95.0 + 110.0) / 3.0; // 106.666...
        let num = tp0 * 1000.0 + tp1 * 2000.0;
        let den = 1000.0 + 2000.0;
        let rvwap = num / den;
        let expected = (110.0_f64 / rvwap).ln();
        assert!(
            (rows[1].vwap_dev - expected).abs() < 1e-12,
            "vwap_dev at index 1 expected {expected}, got {}",
            rows[1].vwap_dev
        );
    }

    #[test]
    fn compute_features_log_return_correct() {
        let c0 = candle(0, 100.0, 110.0, 90.0, 100.0, 1000.0, 100.0);
        let c1 = candle(3600, 100.0, 115.0, 95.0, 110.0, 1200.0, 105.0);
        let rows = compute_features(&[c0, c1]);
        assert_eq!(rows.len(), 2);

        let expected = (110.0_f64 / 100.0).ln();
        assert!(
            (rows[1].log_return - expected).abs() < 1e-12,
            "log_return mismatch: expected {expected}, got {}",
            rows[1].log_return
        );
    }

    #[test]
    fn compute_features_atr_rolling_mean() {
        // Five candles; compute TR manually and verify atr_72 at index 4 = mean(TR[0..=4]).
        let candles = vec![
            candle(0,    100.0, 105.0,  98.0, 103.0, 1000.0, 101.0),
            candle(3600, 103.0, 108.0, 101.0, 106.0, 1100.0, 104.0),
            candle(7200, 106.0, 112.0, 104.0, 109.0, 1200.0, 107.0),
            candle(10800,109.0, 115.0, 107.0, 113.0, 1300.0, 111.0),
            candle(14400,113.0, 120.0, 111.0, 118.0, 1400.0, 115.0),
        ];

        // Manually compute TR for each candle.
        let tr0 = 105.0 - 98.0; // 7.0
        let tr1 = (108.0_f64 - 101.0_f64).max((108.0_f64 - 103.0_f64).abs()).max((101.0_f64 - 103.0_f64).abs()); // max(7,5,2)=7
        let tr2 = (112.0_f64 - 104.0_f64).max((112.0_f64 - 106.0_f64).abs()).max((104.0_f64 - 106.0_f64).abs()); // max(8,6,2)=8
        let tr3 = (115.0_f64 - 107.0_f64).max((115.0_f64 - 109.0_f64).abs()).max((107.0_f64 - 109.0_f64).abs()); // max(8,6,2)=8
        let tr4 = (120.0_f64 - 111.0_f64).max((120.0_f64 - 113.0_f64).abs()).max((111.0_f64 - 113.0_f64).abs()); // max(9,7,2)=9

        let expected_atr = (tr0 + tr1 + tr2 + tr3 + tr4) / 5.0;

        let rows = compute_features(&candles);
        assert!(
            (rows[4].atr_72 - expected_atr).abs() < 1e-12,
            "atr_72 at index 4 expected {expected_atr}, got {}",
            rows[4].atr_72
        );
    }

    #[test]
    fn compute_features_atr_window_clamp() {
        // 80 candles all with TR = 1.0; at index 79 the window clamps to 72 values → mean = 1.0.
        let mut candles = Vec::with_capacity(80);
        // First candle: TR = high - low = 1.0
        candles.push(candle(0, 100.0, 101.0, 100.0, 100.5, 1000.0, 100.25));
        // Remaining candles: set high = prev_close + 0.5, low = prev_close - 0.5,
        // so TR = max(1.0, 0.5, 0.5) = 1.0 for each.
        for i in 1..80_i64 {
            let prev_close = 100.5;
            candles.push(candle(
                i * 3600,
                prev_close,
                prev_close + 0.5,
                prev_close - 0.5,
                prev_close,
                1000.0,
                prev_close,
            ));
        }

        let rows = compute_features(&candles);
        assert_eq!(rows.len(), 80);
        let atr = rows[79].atr_72;
        assert!(
            (atr - 1.0).abs() < 1e-12,
            "atr_72 at index 79 should be 1.0, got {atr}"
        );
    }

    #[test]
    fn compute_features_parity_contract_matches_colab() {
        // Load the parity golden fixture (168h of candles) and assert that the
        // engineered features obey the Colab `SwingTradingDataset` contract:
        //   - vwap_dev == ln(close / rolling_vwap)  (rolling VWAP, NOT the
        //     exchange-reported `vwap` field)
        //   - rolling_vwap == Σ(typical_price*vol)/Σ(vol) over the 72-window
        //   - atr_72 == simple rolling mean of True Range over the 72-window
        use std::path::PathBuf;
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/fixtures/parity_golden_168h.json");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {:?}: {e}", path));
        let v: serde_json::Value = serde_json::from_str(&text).expect("parse fixture");
        let candles_json = v["candles"].as_array().expect("candles array");

        let mut candles = Vec::with_capacity(candles_json.len());
        for c in candles_json {
            candles.push(Candle {
                ts: c["ts"].as_i64().unwrap(),
                open: c["open"].as_f64().unwrap(),
                high: c["high"].as_f64().unwrap(),
                low: c["low"].as_f64().unwrap(),
                close: c["close"].as_f64().unwrap(),
                volume: c["volume"].as_f64().unwrap(),
                vwap: c["vwap"].as_f64().unwrap(),
            });
        }
        assert!(candles.len() >= 80, "fixture too small");

        let rows = compute_features(&candles);

        // Verify at several indices >= 71 (where the full 72-window is available)
        for i in (71..candles.len()).step_by(17) {
            let c = &candles[i];
            let row = &rows[i];

            // Independent recomputation of rolling VWAP over [i-71 .. i].
            let start = i - 71;
            let mut num = 0.0_f64;
            let mut den = 0.0_f64;
            let mut tr_sum = 0.0_f64;
            for j in start..=i {
                let cj = &candles[j];
                let tp = (cj.high + cj.low + cj.close) / 3.0;
                num += tp * cj.volume;
                den += cj.volume;
                // True Range for j
                let tr = if j == 0 {
                    cj.high - cj.low
                } else {
                    let pc = candles[j - 1].close;
                    (cj.high - cj.low)
                        .max((cj.high - pc).abs())
                        .max((cj.low - pc).abs())
                };
                tr_sum += tr;
            }
            let rvwap = num / den;
            let expected_vwap_dev = (c.close / rvwap).ln();
            assert!(
                (row.vwap_dev - expected_vwap_dev).abs() < 1e-9,
                "vwap_dev mismatch at {i}: got {}, expected {expected_vwap_dev}",
                row.vwap_dev
            );

            // ATR is the simple rolling mean of TR over the 72-window.
            let expected_atr = tr_sum / 72.0;
            assert!(
                (row.atr_72 - expected_atr).abs() < 1e-9,
                "atr_72 mismatch at {i}: got {}, expected {expected_atr}",
                row.atr_72
            );

            // Guard: if we accidentally used the candle `vwap` field instead of
            // the rolling VWAP, this would differ. Enforce the rolling definition.
            let wrong = (c.close / c.vwap).ln();
            assert!(
                (row.vwap_dev - wrong).abs() > 1e-6 || (rvwap - c.vwap).abs() < 1e-9,
                "vwap_dev appears to use the candle vwap field at {i} — parity broken"
            );
        }
    }
}
