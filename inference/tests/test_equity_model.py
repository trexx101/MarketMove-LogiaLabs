"""Tests for the Wave C equity inference service (inference/equity_model.py).

Validates:
- TCN architecture matches the trained state_dict (key names, shapes).
- TCN loads from the real qqq_tcn_v1.pt artifact.
- LightGBM models load from the real .pkl artifacts.
- Ensemble produces pred_1d / pred_5d / pred_21d floats.
- V3 wire protocol: valid request, malformed JSON, wrong feature count, empty window.
- Ensemble blending math (weighted average of TCN + LightGBM).
"""
from __future__ import annotations

import json
import math
from pathlib import Path
from unittest.mock import MagicMock

import numpy as np
import pytest
import torch

from inference.equity_model import (
    CausalConv1d,
    EquityEnsemble,
    QqqTCN,
    ResidualBlock,
    _handle_request,
    load_lgbm,
    load_tcn,
)


# ── Fixtures ──────────────────────────────────────────────────────────────────

REPO_ROOT = Path(__file__).resolve().parents[2]
MODELS_DIR = REPO_ROOT / "models"

TCN_PATH = MODELS_DIR / "qqq_tcn_v1.pt"
LGBM_H1_PATH = MODELS_DIR / "qqq_lgbm_h1_v1.pkl"
LGBM_H5_PATH = MODELS_DIR / "qqq_lgbm_h5_v1.pkl"
LGBM_H21_PATH = MODELS_DIR / "qqq_lgbm_h21_v1.pkl"

ARTIFACTS_AVAILABLE = all(p.exists() for p in [TCN_PATH, LGBM_H1_PATH, LGBM_H5_PATH, LGBM_H21_PATH])

SEQ_LEN = 21
N_FEATURES = 8


def _feature_window(seq_len: int = SEQ_LEN, n_features: int = N_FEATURES) -> list[list[float]]:
    """Synthetic normalized feature window."""
    torch.manual_seed(42)
    return torch.randn(seq_len, n_features).tolist()


# ── CausalConv1d tests ────────────────────────────────────────────────────────

class TestCausalConv1d:
    def test_output_length_preserved(self) -> None:
        conv = CausalConv1d(8, 64, kernel_size=3, dilation=1)
        x = torch.randn(1, 8, 50)
        y = conv(x)
        assert y.shape == (1, 64, 50)

    def test_causal_property(self) -> None:
        """Changing future steps must not affect past outputs."""
        conv = CausalConv1d(1, 1, kernel_size=3, dilation=1)
        with torch.no_grad():
            x1 = torch.zeros(1, 1, 10)
            x2 = x1.clone()
            x2[0, 0, 5:] = 99.0
            y1 = conv(x1)
            y2 = conv(x2)
            assert torch.allclose(y1[0, 0, :5], y2[0, 0, :5])


# ── QqqTCN architecture tests ────────────────────────────────────────────────

class TestQqqTCNArchitecture:
    def test_output_is_list_of_3(self) -> None:
        model = QqqTCN(in_dim=8, hidden_dim=64)
        model.eval()
        x = torch.randn(1, SEQ_LEN, 8)
        with torch.no_grad():
            out = model(x)
        assert len(out) == 3

    def test_each_output_shape(self) -> None:
        model = QqqTCN(in_dim=8, hidden_dim=64)
        model.eval()
        x = torch.randn(2, SEQ_LEN, 8)
        with torch.no_grad():
            out = model(x)
        for i, t in enumerate(out):
            assert t.shape == (2,), f"head {i}: expected (2,), got {t.shape}"

    def test_outputs_are_finite(self) -> None:
        model = QqqTCN(in_dim=8, hidden_dim=64)
        model.eval()
        x = torch.randn(1, SEQ_LEN, 8)
        with torch.no_grad():
            out = model(x)
        for t in out:
            assert torch.isfinite(t).all()

    def test_variable_seq_len(self) -> None:
        model = QqqTCN(in_dim=8, hidden_dim=64)
        model.eval()
        for seq in (10, 21, 50, 126):
            x = torch.randn(1, seq, 8)
            with torch.no_grad():
                out = model(x)
            assert out[0].shape == (1,)


# ── Real artifact loading ────────────────────────────────────────────────────

@pytest.mark.skipif(not ARTIFACTS_AVAILABLE, reason="model artifacts not present")
class TestRealArtifactLoading:
    def test_tcn_loads_from_real_checkpoint(self) -> None:
        """The trained qqq_tcn_v1.pt must load without key/shape errors."""
        model = load_tcn(str(TCN_PATH))
        assert isinstance(model, QqqTCN)
        # Verify it produces output
        x = torch.randn(1, SEQ_LEN, 8)
        with torch.no_grad():
            out = model(x)
        assert len(out) == 3
        for t in out:
            assert torch.isfinite(t).all()

    def test_tcn_state_dict_keys_match(self) -> None:
        """State dict keys must match the QqqTCN architecture exactly."""
        sd = torch.load(str(TCN_PATH), map_location="cpu", weights_only=True)
        model = QqqTCN(in_dim=8, hidden_dim=64)
        model_keys = set(model.state_dict().keys())
        ckpt_keys = set(sd.keys())
        assert model_keys == ckpt_keys, (
            f"key mismatch:\n  missing in checkpoint: {model_keys - ckpt_keys}\n"
            f"  extra in checkpoint: {ckpt_keys - model_keys}"
        )

    def test_lgbm_models_load(self) -> None:
        for h, path in [(1, LGBM_H1_PATH), (5, LGBM_H5_PATH), (21, LGBM_H21_PATH)]:
            m = load_lgbm(str(path))
            # load_lgbm returns the raw Booster; verify it can predict on 8 features
            import numpy as np
            row = np.zeros((1, 8), dtype=np.float64)
            pred = m.predict(row)
            assert pred.shape == (1,), f"h{h}: predict returned shape {pred.shape}"


# ── Ensemble tests ────────────────────────────────────────────────────────────

def _mock_ensemble() -> EquityEnsemble:
    """Ensemble with mock models that return known values."""
    tcn = MagicMock(spec=QqqTCN)
    tcn.return_value = [torch.tensor([0.1]), torch.tensor([0.2]), torch.tensor([0.3])]

    lgbm_h1 = MagicMock()
    lgbm_h1.predict.return_value = np.array([0.5])
    lgbm_h5 = MagicMock()
    lgbm_h5.predict.return_value = np.array([0.6])
    lgbm_h21 = MagicMock()
    lgbm_h21.predict.return_value = np.array([0.7])

    return EquityEnsemble(tcn, lgbm_h1, lgbm_h5, lgbm_h21,
                          tcn_weight=0.5, lgbm_weight=0.5)


class TestEquityEnsemble:
    def test_predict_returns_three_keys(self) -> None:
        ens = _mock_ensemble()
        result = ens.predict(_feature_window())
        assert set(result.keys()) == {"pred_1d", "pred_5d", "pred_21d"}

    def test_predict_values_are_floats(self) -> None:
        ens = _mock_ensemble()
        result = ens.predict(_feature_window())
        for v in result.values():
            assert isinstance(v, float)
            assert math.isfinite(v)

    def test_blending_math(self) -> None:
        """Equal weights: result = 0.5*tcn + 0.5*lgbm."""
        ens = _mock_ensemble()
        result = ens.predict(_feature_window())
        assert result["pred_1d"] == pytest.approx(0.5 * 0.1 + 0.5 * 0.5)
        assert result["pred_5d"] == pytest.approx(0.5 * 0.2 + 0.5 * 0.6)
        assert result["pred_21d"] == pytest.approx(0.5 * 0.3 + 0.5 * 0.7)

    def test_custom_weights(self) -> None:
        """TCN-only (weight=1.0) should return TCN values exactly."""
        tcn = MagicMock(spec=QqqTCN)
        tcn.return_value = [torch.tensor([0.1]), torch.tensor([0.2]), torch.tensor([0.3])]
        lgbm_h1 = MagicMock()
        lgbm_h1.predict.return_value = np.array([0.5])
        lgbm_h5 = MagicMock()
        lgbm_h5.predict.return_value = np.array([0.6])
        lgbm_h21 = MagicMock()
        lgbm_h21.predict.return_value = np.array([0.7])

        ens = EquityEnsemble(tcn, lgbm_h1, lgbm_h5, lgbm_h21,
                             tcn_weight=1.0, lgbm_weight=0.0)
        result = ens.predict(_feature_window())
        assert result["pred_1d"] == pytest.approx(0.1)
        assert result["pred_5d"] == pytest.approx(0.2)
        assert result["pred_21d"] == pytest.approx(0.3)


# ── V3 wire protocol tests ────────────────────────────────────────────────────

class TestV3WireContract:
    def _make_req(self, seq_len: int = SEQ_LEN, n_features: int = N_FEATURES) -> bytes:
        return json.dumps({
            "schema_version": 3,
            "feature_window": _feature_window(seq_len, n_features),
        }).encode()

    def test_valid_request_returns_three_floats(self) -> None:
        ens = _mock_ensemble()
        reply = json.loads(_handle_request(self._make_req(), ens, req_id=1))
        assert "pred_1d" in reply
        assert "pred_5d" in reply
        assert "pred_21d" in reply
        for key in ("pred_1d", "pred_5d", "pred_21d"):
            assert isinstance(reply[key], float)
            assert math.isfinite(reply[key])

    def test_malformed_json_returns_error(self) -> None:
        ens = _mock_ensemble()
        reply = json.loads(_handle_request(b"{not valid", ens, req_id=2))
        assert "error" in reply
        assert "json" in reply["error"].lower()

    def test_missing_feature_window_returns_error(self) -> None:
        ens = _mock_ensemble()
        raw = json.dumps({"wrong_key": []}).encode()
        reply = json.loads(_handle_request(raw, ens, req_id=3))
        assert "error" in reply

    def test_empty_feature_window_returns_error(self) -> None:
        ens = _mock_ensemble()
        raw = json.dumps({"feature_window": []}).encode()
        reply = json.loads(_handle_request(raw, ens, req_id=4))
        assert "error" in reply

    def test_wrong_feature_count_returns_error(self) -> None:
        ens = _mock_ensemble()
        raw = json.dumps({
            "schema_version": 3,
            "feature_window": _feature_window(SEQ_LEN, 6),  # 6 instead of 8
        }).encode()
        reply = json.loads(_handle_request(raw, ens, req_id=5))
        assert "error" in reply
        assert "8" in reply["error"]

    def test_different_seq_lengths_all_succeed(self) -> None:
        ens = _mock_ensemble()
        for seq_len in (10, 21, 50, 126):
            raw = self._make_req(seq_len=seq_len)
            reply = json.loads(_handle_request(raw, ens, req_id=seq_len))
            assert "pred_1d" in reply, f"seq_len={seq_len} failed"


# ── End-to-end with real artifacts ────────────────────────────────────────────

@pytest.mark.skipif(not ARTIFACTS_AVAILABLE, reason="model artifacts not present")
class TestRealEnsembleEndToEnd:
    def test_real_ensemble_predicts(self) -> None:
        """Load real artifacts and run a prediction end-to-end."""
        tcn = load_tcn(str(TCN_PATH))
        lgbm_h1 = load_lgbm(str(LGBM_H1_PATH))
        lgbm_h5 = load_lgbm(str(LGBM_H5_PATH))
        lgbm_h21 = load_lgbm(str(LGBM_H21_PATH))

        ens = EquityEnsemble(tcn, lgbm_h1, lgbm_h5, lgbm_h21)
        result = ens.predict(_feature_window())

        assert set(result.keys()) == {"pred_1d", "pred_5d", "pred_21d"}
        for key, val in result.items():
            assert isinstance(val, float), f"{key} not float: {type(val)}"
            assert math.isfinite(val), f"{key} not finite: {val}"

    def test_real_ensemble_via_wire_protocol(self) -> None:
        """Full wire protocol round-trip with real models."""
        tcn = load_tcn(str(TCN_PATH))
        lgbm_h1 = load_lgbm(str(LGBM_H1_PATH))
        lgbm_h5 = load_lgbm(str(LGBM_H5_PATH))
        lgbm_h21 = load_lgbm(str(LGBM_H21_PATH))

        ens = EquityEnsemble(tcn, lgbm_h1, lgbm_h5, lgbm_h21)
        raw = json.dumps({
            "schema_version": 3,
            "feature_window": _feature_window(),
        }).encode()
        reply = json.loads(_handle_request(raw, ens, req_id=1))

        assert "pred_1d" in reply
        assert "pred_5d" in reply
        assert "pred_21d" in reply
        for key in ("pred_1d", "pred_5d", "pred_21d"):
            assert isinstance(reply[key], float)
            assert math.isfinite(reply[key])
