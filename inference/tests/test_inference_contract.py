"""Unit tests for the Feature-04 inference contract.

Uses randomly initialised weights so no ``model.pt`` artifact is required.
Validates:
- Output shape (three float predictions).
- Output scaling (values in log-return range, not the ×100 training space).
- JSON REQ/REP wire protocol via ``_handle_request``.
- Error paths (malformed JSON, wrong input shape).
"""
from __future__ import annotations

import json
import math
import tempfile
from pathlib import Path

import pytest
import torch

from inference.model import CausalConv1d, MarketMarkovNet, load_model
from inference.inference_engine import _handle_request, _tensorize


# ── Helpers ───────────────────────────────────────────────────────────────────

N_FEATURES = 3
SEQ_LEN = 72


def _random_model(input_features: int = N_FEATURES, hidden_dim: int = 32, rank: int = 8) -> MarketMarkovNet:
    """Return a randomly-initialised model (no file I/O needed)."""
    model = MarketMarkovNet(input_features=input_features, hidden_dim=hidden_dim, rank=rank)
    model.eval()
    return model


def _feature_window(seq_len: int = SEQ_LEN, n_features: int = N_FEATURES) -> list[list[float]]:
    """Return a synthetic feature window as a plain Python list."""
    t = torch.randn(seq_len, n_features)
    return t.tolist()


# ── CausalConv1d tests ────────────────────────────────────────────────────────

class TestCausalConv1d:
    def test_output_length_preserved(self) -> None:
        conv = CausalConv1d(3, 8, kernel_size=3, dilation=1)
        x = torch.randn(1, 3, 50)
        y = conv(x)
        assert y.shape == (1, 8, 50), f"expected (1, 8, 50), got {y.shape}"

    def test_output_length_preserved_large_dilation(self) -> None:
        conv = CausalConv1d(8, 8, kernel_size=3, dilation=16)
        x = torch.randn(2, 8, 100)
        y = conv(x)
        assert y.shape == (2, 8, 100)

    def test_causal_property(self) -> None:
        """Prediction at step t must not change when future steps change."""
        conv = CausalConv1d(1, 1, kernel_size=3, dilation=1)
        with torch.no_grad():
            x1 = torch.zeros(1, 1, 10)
            x2 = x1.clone()
            x2[0, 0, 5:] = 99.0  # change future only
            y1 = conv(x1)
            y2 = conv(x2)
            # Steps before index 5 must be identical
            assert torch.allclose(y1[0, 0, :5], y2[0, 0, :5])


# ── MarketMarkovNet shape tests ───────────────────────────────────────────────

class TestMarketMarkovNetShapes:
    def test_output_tuple_length(self) -> None:
        model = _random_model()
        x = torch.randn(1, SEQ_LEN, N_FEATURES)
        with torch.no_grad():
            out = model(x)
        assert len(out) == 3, "expected 3-tuple (pred_1h, pred_4h, pred_24h)"

    def test_each_output_shape(self) -> None:
        model = _random_model()
        x = torch.randn(2, SEQ_LEN, N_FEATURES)
        with torch.no_grad():
            p1h, p4h, p24h = model(x)
        for name, t in [("pred_1h", p1h), ("pred_4h", p4h), ("pred_24h", p24h)]:
            assert t.shape == (2, 1), f"{name}: expected (2, 1), got {t.shape}"

    def test_outputs_are_finite(self) -> None:
        model = _random_model()
        x = torch.randn(1, SEQ_LEN, N_FEATURES)
        with torch.no_grad():
            p1h, p4h, p24h = model(x)
        for t in (p1h, p4h, p24h):
            assert torch.isfinite(t).all(), "output contains NaN or Inf"

    def test_output_scaling_lt_1(self) -> None:
        """Predictions should be in log-return scale (~small floats), not ×100."""
        torch.manual_seed(0)
        model = _random_model()
        # Zero input → near-zero predictions (biases only, likely small)
        x = torch.zeros(1, SEQ_LEN, N_FEATURES)
        with torch.no_grad():
            p1h, p4h, p24h = model(x)
        for name, t in [("pred_1h", p1h), ("pred_4h", p4h), ("pred_24h", p24h)]:
            val = float(t.squeeze())
            assert abs(val) < 10.0, (
                f"{name}={val:.4f} looks un-scaled (> 10.0); "
                "check the /100 division in MarketMarkovNet.forward"
            )

    def test_variable_seq_len(self) -> None:
        model = _random_model()
        for seq in (24, 72, 168):
            x = torch.randn(1, seq, N_FEATURES)
            with torch.no_grad():
                p1h, p4h, p24h = model(x)
            assert p1h.shape == (1, 1)

    def test_batch_size_independence(self) -> None:
        """Predictions for the same input must not change across batch sizes."""
        model = _random_model()
        single = torch.randn(1, SEQ_LEN, N_FEATURES)
        batched = single.expand(4, -1, -1)
        with torch.no_grad():
            p_single = model(single)
            p_batched = model(batched)
        for s, b in zip(p_single, p_batched):
            assert torch.allclose(s, b[0:1], atol=1e-5)


# ── Checkpoint round-trip ────────────────────────────────────────────────────

class TestCheckpointRoundTrip:
    def test_save_load_produces_identical_output(self) -> None:
        torch.manual_seed(42)
        model = _random_model()
        x = torch.randn(1, SEQ_LEN, N_FEATURES)

        with torch.no_grad():
            orig_out = model(x)

        with tempfile.NamedTemporaryFile(suffix=".pt", delete=False) as f:
            tmp_path = f.name

        torch.save(model.state_dict(), tmp_path)
        loaded = load_model(tmp_path, input_features=N_FEATURES, hidden_dim=32, rank=8)

        with torch.no_grad():
            loaded_out = loaded(x)

        for a, b in zip(orig_out, loaded_out):
            assert torch.allclose(a, b, atol=1e-6), "loaded model output differs"

        Path(tmp_path).unlink(missing_ok=True)


# ── _tensorize ────────────────────────────────────────────────────────────────

class TestTensorize:
    def test_shape(self) -> None:
        fw = _feature_window(SEQ_LEN, N_FEATURES)
        t = _tensorize(fw, torch.device("cpu"))
        assert t.shape == (1, SEQ_LEN, N_FEATURES)

    def test_dtype(self) -> None:
        fw = _feature_window()
        t = _tensorize(fw, torch.device("cpu"))
        assert t.dtype == torch.float32


# ── REQ/REP wire contract ─────────────────────────────────────────────────────

class TestWireContract:
    def _make_req(self, seq_len: int = SEQ_LEN, n_features: int = N_FEATURES) -> bytes:
        return json.dumps({"feature_window": _feature_window(seq_len, n_features)}).encode()

    def test_valid_request_returns_three_floats(self) -> None:
        model = _random_model()
        raw = self._make_req()
        reply_bytes = _handle_request(raw, model, req_id=1)
        reply = json.loads(reply_bytes)
        assert "pred_1h" in reply
        assert "pred_4h" in reply
        assert "pred_24h" in reply
        for key in ("pred_1h", "pred_4h", "pred_24h"):
            assert isinstance(reply[key], float)
            assert math.isfinite(reply[key])

    def test_predictions_are_log_return_scale(self) -> None:
        """All predictions must be well within log-return magnitude bounds."""
        model = _random_model()
        raw = self._make_req()
        reply = json.loads(_handle_request(raw, model, req_id=1))
        for key in ("pred_1h", "pred_4h", "pred_24h"):
            assert abs(reply[key]) < 10.0, f"{key}={reply[key]} exceeds reasonable log-return range"

    def test_malformed_json_returns_error(self) -> None:
        model = _random_model()
        reply = json.loads(_handle_request(b"{not valid json", model, req_id=2))
        assert "error" in reply
        assert "json" in reply["error"].lower()

    def test_missing_feature_window_returns_error(self) -> None:
        model = _random_model()
        raw = json.dumps({"wrong_key": []}).encode()
        reply = json.loads(_handle_request(raw, model, req_id=3))
        assert "error" in reply

    def test_empty_feature_window_returns_error(self) -> None:
        model = _random_model()
        raw = json.dumps({"feature_window": []}).encode()
        reply = json.loads(_handle_request(raw, model, req_id=4))
        assert "error" in reply

    def test_different_seq_lengths_all_succeed(self) -> None:
        model = _random_model()
        for seq_len in (24, 48, 72, 168):
            raw = self._make_req(seq_len=seq_len)
            reply = json.loads(_handle_request(raw, model, req_id=seq_len))
            assert "pred_1h" in reply, f"seq_len={seq_len} failed"
