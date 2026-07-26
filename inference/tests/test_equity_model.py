"""Round-trip contract tests for the V3 equities inference service.

Tests the JSON wire protocol: V3 request → ZMQ REP → V3 response.
These are integration-style tests that import the actual service modules
(and thus need real model artifacts on disk).  They are skipped when
artifacts are absent so they don't break in environments without a
models/ directory.
"""
from __future__ import annotations

import json
import os
import socket
import subprocess
import sys
import time
from pathlib import Path

import pytest

# Artifact paths — these must match deploy/config.md defaults.
MODELS_DIR = Path(__file__).resolve().parents[2] / "models"
TCN_PATH = MODELS_DIR / "qqq_tcn_v1.pt"
LGBM_H1 = MODELS_DIR / "qqq_lgbm_h1_v1.pkl"
LGBM_H5 = MODELS_DIR / "qqq_lgbm_h5_v1.pkl"
LGBM_H21 = MODELS_DIR / "qqq_lgbm_h21_v1.pkl"


def _artifacts_present() -> bool:
    return all(p.exists() for p in (TCN_PATH, LGBM_H1, LGBM_H5, LGBM_H21))


SKIP_IF_NO_ARTIFACTS = pytest.mark.skipif(
    not _artifacts_present(),
    reason="V3 model artifacts not found — set up models/ to run these tests",
)


# ── V3 request/response helpers ──────────────────────────────────────────────


def _free_port() -> int:
    """Return an unused TCP port (useful for binding test servers)."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def _v3_request(schema_version: int, feature_window: list[list[float]],
                 atr_ratio: float | None = None) -> dict:
    req = {
        "schema_version": schema_version,
        "feature_window": feature_window,
    }
    if atr_ratio is not None:
        req["atr_ratio"] = atr_ratio
    return req


# ── Ensemble unit tests (no ZMQ) ──────────────────────────────────────────────


@SKIP_IF_NO_ARTIFACTS
def test_ensemble_predict_returns_3_horizons() -> None:
    """EquityEnsemble.predict must return pred_1d, pred_5d, pred_21d."""
    import torch  # noqa: F401  (presence check)
    from inference.equity_model import EquityEnsemble, load_lgbm, load_tcn

    tcn = load_tcn(str(TCN_PATH))
    lgbm_h1 = load_lgbm(str(LGBM_H1))
    lgbm_h5 = load_lgbm(str(LGBM_H5))
    lgbm_h21 = load_lgbm(str(LGBM_H21))

    ensemble = EquityEnsemble(tcn, lgbm_h1, lgbm_h5, lgbm_h21)

    # Valid 8-dim window with 126 timesteps (matches FEATURE_WINDOW_SIZE=126)
    window = [[0.0] * 8 for _ in range(126)]
    preds = ensemble.predict(window, atr_ratio=0.005)

    assert isinstance(preds, dict), "predict must return a dict"
    assert set(preds.keys()) == {"pred_1d", "pred_5d", "pred_21d"}, (
        f"expected pred_1d/pred_5d/pred_21d, got {set(preds.keys())}"
    )
    for key, val in preds.items():
        assert isinstance(val, float), f"{key} must be float, got {type(val).__name__}"
        assert val == val, f"{key} must not be NaN"  # noqa: PLR0133


@SKIP_IF_NO_ARTIFACTS
def test_ensemble_predict_with_various_window_sizes() -> None:
    """Ensemble must handle windows longer than seq_len (TCN reads last timesteps)."""
    import torch  # noqa: F401
    from inference.equity_model import EquityEnsemble, load_lgbm, load_tcn

    tcn = load_tcn(str(TCN_PATH))
    lgbm_h1 = load_lgbm(str(LGBM_H1))
    lgbm_h5 = load_lgbm(str(LGBM_H5))
    lgbm_h21 = load_lgbm(str(LGBM_H21))
    ensemble = EquityEnsemble(tcn, lgbm_h1, lgbm_h5, lgbm_h21)

    for seq_len in (1, 5, 21, 72, 126):
        window = [[0.0] * 8 for _ in range(seq_len)]
        preds = ensemble.predict(window, atr_ratio=0.005)
        assert set(preds.keys()) == {"pred_1d", "pred_5d", "pred_21d"}, f"failed at seq_len={seq_len}"


@SKIP_IF_NO_ARTIFACTS
def test_ensemble_blend_logic_and_atr_denorm() -> None:
    """Verify blend weights and ATR denormalization via isolated computation.

    The blend: label = w_t * tcn + w_l * lgbm
    The denorm: raw = label * atr_ratio
    We mock the model outputs to test the arithmetic in isolation.
    """
    import torch  # noqa: F401
    import numpy as np
    from inference.equity_model import EquityEnsemble

    # Mock TCN and LightGBM that return known values
    class MockTCN:
        def __call__(self, x):
            # Return (1, 1, 1) tensor for 3 horizons
            return [torch.tensor([[0.6]], dtype=torch.float32),
                    torch.tensor([[0.2]], dtype=torch.float32),
                    torch.tensor([[0.1]], dtype=torch.float32)]

    class MockLGBM:
        def __init__(self, value):
            self.value = value
        def predict(self, x):
            return [self.value]

    # Test case: equal blend, atr_ratio=0.005
    ensemble = EquityEnsemble(
        MockTCN(), MockLGBM(0.4), MockLGBM(0.0), MockLGBM(-0.1),
        tcn_weight=0.5, lgbm_weight=0.5,
    )
    # Use any valid-length window
    window = [[0.0] * 8 for _ in range(126)]
    preds = ensemble.predict(window, atr_ratio=0.005)

    # Check each horizon manually:
    # pred_1d:  label = 0.5*0.6 + 0.5*0.4 = 0.5,  raw = 0.5 * 0.005 = 0.0025
    # pred_5d:  label = 0.5*0.2 + 0.5*0.0 = 0.1,  raw = 0.1  * 0.005 = 0.0005
    # pred_21d: label = 0.5*0.1 + 0.5*(-0.1)= 0.0, raw = 0.0 * 0.005 = 0.0
    assert np.isclose(preds["pred_1d"], 0.0025), f"pred_1d={preds['pred_1d']}"
    assert np.isclose(preds["pred_5d"], 0.0005), f"pred_5d={preds['pred_5d']}"
    assert np.isclose(preds["pred_21d"], 0.0), f"pred_21d={preds['pred_21d']}"

    # Test: asymmetric weights shift prediction
    ensemble_asym = EquityEnsemble(
        MockTCN(), MockLGBM(0.4), MockLGBM(0.0), MockLGBM(-0.1),
        tcn_weight=1.0, lgbm_weight=0.0,
    )
    preds_asym = ensemble_asym.predict(window, atr_ratio=0.005)
    # pred_1d: label = 1.0*0.6 + 0.0*0.4 = 0.6, raw = 0.6 * 0.005 = 0.003
    assert np.isclose(preds_asym["pred_1d"], 0.003), f"pred_1d={preds_asym['pred_1d']}"
    # Must differ from equal blend
    assert preds_asym["pred_1d"] != preds["pred_1d"]

    # Test: atr_ratio scales output
    preds_hi = ensemble.predict(window, atr_ratio=0.01)
    assert np.isclose(preds_hi["pred_1d"], 0.005), f"pred_1d={preds_hi['pred_1d']}"  # 0.5 * 0.01
    assert np.isclose(preds_hi["pred_5d"], 0.001), f"pred_5d={preds_hi['pred_5d']}"  # 0.1 * 0.01


# ── Handle-request contract tests ─────────────────────────────────────────────


def _handle_request_v3(
    feature_window: list[list[float]],
    atr_ratio: float = 0.005,
) -> dict[str, float]:
    """Call _handle_request directly (bypasses ZMQ) to test the handler logic."""
    import torch  # noqa: F401
    from inference.equity_model import EquityEnsemble, load_lgbm, load_tcn, _handle_request

    tcn = load_tcn(str(TCN_PATH))
    lgbm_h1 = load_lgbm(str(LGBM_H1))
    lgbm_h5 = load_lgbm(str(LGBM_H5))
    lgbm_h21 = load_lgbm(str(LGBM_H21))
    ensemble = EquityEnsemble(tcn, lgbm_h1, lgbm_h5, lgbm_h21)

    request = _v3_request(schema_version=3, feature_window=feature_window, atr_ratio=atr_ratio)
    raw = json.dumps(request).encode()
    reply_raw = _handle_request(raw, ensemble, req_id=1)
    return json.loads(reply_raw.decode())


@SKIP_IF_NO_ARTIFACTS
def test_handle_request_returns_correct_keys() -> None:
    """_handle_request must return pred_1d, pred_5d, pred_21d in log-return space."""
    window = [[0.0] * 8 for _ in range(126)]
    atr_ratio = 0.005
    reply = _handle_request_v3(window, atr_ratio=atr_ratio)
    assert set(reply.keys()) == {"pred_1d", "pred_5d", "pred_21d"}, f"unexpected keys: {set(reply.keys())}"
    # After ATR denormalization, predictions should be small (order of atr_ratio)
    for key, val in reply.items():
        assert abs(val) < atr_ratio * 5, f"{key}={val} too large for atr_ratio={atr_ratio}"


def test_handle_request_atr_ratio_0_is_near_zero() -> None:
    """atr_ratio=0 should produce near-zero predictions (no signal)."""
    window = [[0.0] * 8 for _ in range(126)]
    reply = _handle_request_v3(window, atr_ratio=0.0)
    assert set(reply.keys()) == {"pred_1d", "pred_5d", "pred_21d"}
    for val in reply.values():
        assert abs(val) < 1e-9, f"atr_ratio=0 should yield near-zero, got {val}"


def test_handle_request_atr_ratio_scales_output() -> None:
    """Higher atr_ratio should scale up predictions linearly."""
    # Varying features so the model produces non-constant predictions
    window = [[(i + j) * 0.1 for j in range(8)] for i in range(126)]
    r0 = _handle_request_v3(window, atr_ratio=0.0)
    r5 = _handle_request_v3(window, atr_ratio=0.005)
    for key in r0:
        # Positive ATR should produce larger absolute predictions than zero ATR
        assert abs(r5[key]) > abs(r0[key]), f"{key}: expected scaled prediction"


@SKIP_IF_NO_ARTIFACTS
def test_handle_request_rejects_wrong_feature_dim() -> None:
    """Handler must reject windows with wrong feature dimension."""
    from inference.equity_model import EquityEnsemble, load_lgbm, load_tcn, _handle_request

    tcn = load_tcn(str(TCN_PATH))
    lgbm_h1 = load_lgbm(str(LGBM_H1))
    lgbm_h5 = load_lgbm(str(LGBM_H5))
    lgbm_h21 = load_lgbm(str(LGBM_H21))
    ensemble = EquityEnsemble(tcn, lgbm_h1, lgbm_h5, lgbm_h21)

    # 3 features (old V1 shape) — must be rejected
    request = _v3_request(schema_version=3, feature_window=[[0.0] * 3 for _ in range(126)])
    raw = json.dumps(request).encode()
    reply_raw = _handle_request(raw, ensemble, req_id=1)
    reply = json.loads(reply_raw.decode())
    assert "error" in reply, "wrong feature dim must return an error"


@SKIP_IF_NO_ARTIFACTS
def test_handle_request_rejects_empty_window() -> None:
    """Handler must reject empty feature_window."""
    from inference.equity_model import EquityEnsemble, load_lgbm, load_tcn, _handle_request

    tcn = load_tcn(str(TCN_PATH))
    lgbm_h1 = load_lgbm(str(LGBM_H1))
    lgbm_h5 = load_lgbm(str(LGBM_H5))
    lgbm_h21 = load_lgbm(str(LGBM_H21))
    ensemble = EquityEnsemble(tcn, lgbm_h1, lgbm_h5, lgbm_h21)

    request = _v3_request(schema_version=3, feature_window=[])
    raw = json.dumps(request).encode()
    reply_raw = _handle_request(raw, ensemble, req_id=1)
    reply = json.loads(reply_raw.decode())
    assert "error" in reply, "empty window must return an error"


@SKIP_IF_NO_ARTIFACTS
def test_handle_request_rejects_invalid_json() -> None:
    """Handler must return error for unparseable JSON."""
    from inference.equity_model import EquityEnsemble, load_lgbm, load_tcn, _handle_request

    tcn = load_tcn(str(TCN_PATH))
    lgbm_h1 = load_lgbm(str(LGBM_H1))
    lgbm_h5 = load_lgbm(str(LGBM_H5))
    lgbm_h21 = load_lgbm(str(LGBM_H21))
    ensemble = EquityEnsemble(tcn, lgbm_h1, lgbm_h5, lgbm_h21)

    reply_raw = _handle_request(b"not valid json {{{", ensemble, req_id=1)
    reply = json.loads(reply_raw.decode())
    assert "error" in reply, "invalid JSON must return an error"


# ── Config tests ─────────────────────────────────────────────────────────────


def test_equity_inference_config_defaults() -> None:
    """EquityInferenceConfig.from_env must resolve defaults correctly."""
    import os
    from inference.config import EquityInferenceConfig

    # Clear any pre-existing env vars
    for key in ("TCN_PATH", "LGBM_H1_PATH", "LGBM_H5_PATH", "LGBM_H21_PATH",
                "ZMQ_BIND", "ZMQ_ENDPOINT", "MODELS_DIR", "TCN_WEIGHT", "LGBM_WEIGHT"):
        os.environ.pop(key, None)

    cfg = EquityInferenceConfig.from_env()
    assert cfg.zmq_bind == "tcp://*:5555"
    assert cfg.tcn_path.name == "qqq_tcn_v1.pt"
    assert cfg.lgbm_h1_path.name == "qqq_lgbm_h1_v1.pkl"
    assert cfg.lgbm_h5_path.name == "qqq_lgbm_h5_v1.pkl"
    assert cfg.lgbm_h21_path.name == "qqq_lgbm_h21_v1.pkl"
    assert cfg.tcn_weight == 0.5
    assert cfg.lgbm_weight == 0.5


def test_equity_inference_config_env_override() -> None:
    """EquityInferenceConfig.from_env must respect env var overrides."""
    import os
    from inference.config import EquityInferenceConfig

    for key in ("TCN_PATH", "LGBM_H1_PATH", "LGBM_H5_PATH", "LGBM_H21_PATH",
                "ZMQ_BIND", "ZMQ_ENDPOINT", "TCN_WEIGHT", "LGBM_WEIGHT"):
        os.environ.pop(key, None)

    os.environ["TCN_PATH"] = "/custom/tcn.pt"
    os.environ["LGBM_H1_PATH"] = "/custom/h1.pkl"
    os.environ["LGBM_H5_PATH"] = "/custom/h5.pkl"
    os.environ["LGBM_H21_PATH"] = "/custom/h21.pkl"
    os.environ["ZMQ_BIND"] = "tcp://127.0.0.1:9999"
    os.environ["TCN_WEIGHT"] = "0.7"
    os.environ["LGBM_WEIGHT"] = "0.3"

    try:
        cfg = EquityInferenceConfig.from_env()
        assert cfg.tcn_path == Path("/custom/tcn.pt")
        assert cfg.lgbm_h1_path == Path("/custom/h1.pkl")
        assert cfg.lgbm_h5_path == Path("/custom/h5.pkl")
        assert cfg.lgbm_h21_path == Path("/custom/h21.pkl")
        assert cfg.zmq_bind == "tcp://127.0.0.1:9999"
        assert cfg.tcn_weight == 0.7
        assert cfg.lgbm_weight == 0.3
    finally:
        for key in ("TCN_PATH", "LGBM_H1_PATH", "LGBM_H5_PATH", "LGBM_H21_PATH",
                    "ZMQ_BIND", "ZMQ_ENDPOINT", "TCN_WEIGHT", "LGBM_WEIGHT"):
            os.environ.pop(key, None)


def test_equity_inference_config_require_artifacts_missing() -> None:
    """require_artifacts must raise FileNotFoundError for missing files."""
    import os
    from inference.config import EquityInferenceConfig

    for key in ("TCN_PATH", "LGBM_H1_PATH", "LGBM_H5_PATH", "LGBM_H21_PATH"):
        os.environ.pop(key, None)

    cfg = EquityInferenceConfig(
        zmq_endpoint="tcp://127.0.0.1:5555",
        zmq_bind="tcp://*:5555",
        tcn_path=Path("/does/not/exist/tcn.pt"),
        lgbm_h1_path=Path("/does/not/exist/h1.pkl"),
        lgbm_h5_path=Path("/does/not/exist/h5.pkl"),
        lgbm_h21_path=Path("/does/not/exist/h21.pkl"),
        tcn_weight=0.5,
        lgbm_weight=0.5,
    )
    with pytest.raises(FileNotFoundError, match="V3 artifacts not found"):
        cfg.require_artifacts()


# ── Full-process integration test (spawns the service) ────────────────────────


@SKIP_IF_NO_ARTIFACTS
@pytest.mark.slow
def test_service_end_to_end_zmq_roundtrip() -> None:
    """Start the service, send a V3 request over ZMQ, verify the response."""
    port = _free_port()
    bind_addr = f"tcp://127.0.0.1:{port}"

    env = {
        **os.environ,
        "TCN_PATH": str(TCN_PATH),
        "LGBM_H1_PATH": str(LGBM_H1),
        "LGBM_H5_PATH": str(LGBM_H5),
        "LGBM_H21_PATH": str(LGBM_H21),
        "ZMQ_BIND": bind_addr,
    }

    proc = subprocess.Popen(
        [sys.executable, "-m", "inference.equity_model"],
        cwd=Path(__file__).resolve().parents[2],
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )

    try:
        # Wait for the service to bind and load models (up to 60s).
        import zmq
        ctx = zmq.Context()
        sock = ctx.socket(zmq.REQ)
        sock.setsockopt(zmq.LINGER, 0)
        sock.setsockopt(zmq.RCVTIMEO, 60_000)
        sock.setsockopt(zmq.SNDTIMEO, 10_000)

        connected = False
        for _ in range(60):
            try:
                sock.connect(bind_addr)
                connected = True
                break
            except zmq.ZMQError:
                time.sleep(1)
        assert connected, "could not connect to service within 60s"

        # Send a V3 request (with atr_ratio)
        request = _v3_request(schema_version=3, feature_window=[[0.0] * 8 for _ in range(126)],
                             atr_ratio=0.005)
        sock.send(json.dumps(request).encode())
        reply_raw = sock.recv()
        reply = json.loads(reply_raw.decode())

        # Verify response shape
        assert "error" not in reply, f"service returned error: {reply.get('error')}"
        assert set(reply.keys()) == {"pred_1d", "pred_5d", "pred_21d"}
        for key, val in reply.items():
            assert isinstance(val, (int, float)), f"{key} must be numeric"
            assert val == val, f"{key} must not be NaN"

        ctx.destroy()
    finally:
        proc.terminate()
        stdout, stderr = proc.communicate(timeout=10)
        if proc.returncode not in (0, -15):  # -15 = SIGTERM
            pytest.fail(
                f"service exited with code {proc.returncode}\n"
                f"stdout: {stdout.decode(errors='replace')}\n"
                f"stderr: {stderr.decode(errors='replace')}"
            )
