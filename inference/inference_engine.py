"""MarketMarkovNet inference microservice (ZMQ REP).

Binds a ZeroMQ REP socket and serves prediction requests over a JSON
REQ/REP protocol:

  Request  → { "feature_window": [[f0, f1, ...], ...] }   # (seq_len × n_features)
  Response → { "pred_1h": float, "pred_4h": float, "pred_24h": float }

Every request/response pair is emitted to stdout as a single JSON line for
parity auditing.  Graceful shutdown is handled via SIGINT / SIGTERM.
"""
from __future__ import annotations

import json
import logging
import os
import signal
import sys
import time
from datetime import datetime, timezone
from typing import Any

import torch

from .config import InferenceConfig
from .model import MarketMarkovNet, load_model


# ── Structured JSON logger ────────────────────────────────────────────────────

class _JsonFormatter(logging.Formatter):
    """Emit each log record as a single JSON line."""

    def format(self, record: logging.LogRecord) -> str:  # noqa: A003
        payload: dict[str, Any] = {
            "ts": datetime.now(timezone.utc).isoformat(),
            "level": record.levelname,
            "msg": record.getMessage(),
        }
        if record.exc_info:
            payload["exc"] = self.formatException(record.exc_info)
        return json.dumps(payload, separators=(",", ":"))


def _build_logger() -> logging.Logger:
    logger = logging.getLogger("inference")
    logger.setLevel(logging.INFO)
    if not logger.handlers:
        handler = logging.StreamHandler(sys.stdout)
        handler.setFormatter(_JsonFormatter())
        logger.addHandler(handler)
    return logger


log = _build_logger()


# ── Request / response handling ───────────────────────────────────────────────

def _tensorize(feature_window: list[list[float]], device: torch.device) -> torch.Tensor:
    """Convert a nested list (seq_len × n_features) to a model-ready tensor.

    Returns shape ``(1, seq_len, n_features)`` (batch=1, sequence-first).
    The model transposes to channels-first internally.
    """
    t = torch.tensor(feature_window, dtype=torch.float32, device=device)
    # t: (seq_len, n_features) → (1, seq_len, n_features)
    return t.unsqueeze(0)


def _handle_request(
    raw_bytes: bytes,
    model: MarketMarkovNet,
    req_id: int,
) -> bytes:
    """Decode one REQ message, run inference, and return the serialized reply."""
    try:
        request: dict[str, Any] = json.loads(raw_bytes)
    except json.JSONDecodeError as exc:
        err_resp = {"error": f"json decode error: {exc}"}
        log.error("req_id=%d json_error=%s", req_id, str(exc))
        return json.dumps(err_resp).encode()

    feature_window = request.get("feature_window")
    if (
        not isinstance(feature_window, list)
        or not feature_window
        or not isinstance(feature_window[0], list)
    ):
        err_resp = {"error": "feature_window must be a non-empty list of lists"}
        log.error("req_id=%d invalid_input", req_id)
        return json.dumps(err_resp).encode()

    try:
        x = _tensorize(feature_window, device=torch.device("cpu"))
        with torch.no_grad():
            p1h, p4h, p24h = model(x)

        pred_1h = float(p1h.squeeze())
        pred_4h = float(p4h.squeeze())
        pred_24h = float(p24h.squeeze())
    except Exception as exc:  # noqa: BLE001
        err_resp = {"error": f"inference error: {exc}"}
        log.error("req_id=%d inference_error=%s", req_id, str(exc))
        return json.dumps(err_resp).encode()

    response = {
        "pred_1h": pred_1h,
        "pred_4h": pred_4h,
        "pred_24h": pred_24h,
    }

    log.info(
        json.dumps(
            {
                "req_id": req_id,
                "seq_len": len(feature_window),
                "n_features": len(feature_window[0]),
                "pred_1h": pred_1h,
                "pred_4h": pred_4h,
                "pred_24h": pred_24h,
            },
            separators=(",", ":"),
        )
    )

    return json.dumps(response).encode()


# ── Service entry point ───────────────────────────────────────────────────────

def _wait_for_model(model_path: str, max_wait: int = 60) -> None:
    """Block until ``model_path`` exists (useful for Docker startup ordering)."""
    import pathlib

    deadline = time.monotonic() + max_wait
    p = pathlib.Path(model_path)
    while not p.exists():
        if time.monotonic() >= deadline:
            raise FileNotFoundError(
                f"Model artifact not found after {max_wait}s: {model_path}"
            )
        log.info("waiting for model artifact: %s", model_path)
        time.sleep(2)


def run_service(cfg: InferenceConfig) -> int:
    """Start the ZMQ REP loop.  Blocks until a shutdown signal is received."""
    try:
        import zmq  # noqa: PLC0415  (deferred import — only needed at runtime)
    except ImportError:
        log.error("pyzmq is not installed; cannot start inference service")
        return 1

    # ── Graceful shutdown flag ────────────────────────────────────────────────
    _stop = {"flag": False}

    def _handle_signal(signum: int, _frame: object) -> None:
        log.info("received signal %d — shutting down", signum)
        _stop["flag"] = True

    signal.signal(signal.SIGINT, _handle_signal)
    signal.signal(signal.SIGTERM, _handle_signal)

    # ── Load model ────────────────────────────────────────────────────────────
    log.info("loading model from %s", cfg.model_path)
    try:
        input_features = int(os.environ.get("MODEL_INPUT_FEATURES", "3"))
        hidden_dim = int(os.environ.get("MODEL_HIDDEN_DIM", "64"))
        rank = int(os.environ.get("MODEL_RANK", "8"))
        model = load_model(
            str(cfg.model_path),
            input_features=input_features,
            hidden_dim=hidden_dim,
            rank=rank,
        )
    except Exception as exc:  # noqa: BLE001
        log.error("failed to load model: %s", exc)
        return 1

    log.info(
        "model loaded — parameters=%d",
        sum(p.numel() for p in model.parameters()),
    )

    # ── Bind ZMQ socket ───────────────────────────────────────────────────────
    context = zmq.Context()
    socket = context.socket(zmq.REP)
    try:
        socket.bind(cfg.zmq_bind)
    except zmq.ZMQError as exc:
        log.error("failed to bind ZMQ socket %s: %s", cfg.zmq_bind, exc)
        context.destroy()
        return 1

    log.info("ZMQ REP bound to %s — ready", cfg.zmq_bind)

    # ── REQ/REP loop ──────────────────────────────────────────────────────────
    req_id = 0
    poller = zmq.Poller()
    poller.register(socket, zmq.POLLIN)

    while not _stop["flag"]:
        try:
            events = dict(poller.poll(timeout=500))  # 500 ms tick
        except zmq.ZMQError:
            break

        if socket not in events:
            continue  # no message yet — check stop flag and loop

        try:
            raw = socket.recv(flags=zmq.NOBLOCK)
        except zmq.Again:
            continue

        req_id += 1
        reply = _handle_request(raw, model, req_id)
        socket.send(reply)

    log.info("shutting down — processed %d requests", req_id)
    socket.close()
    context.term()
    return 0


def main() -> int:
    cfg = InferenceConfig.from_env()
    log.info("inference configured: %s", cfg.summary())
    try:
        cfg.require_artifacts()
    except FileNotFoundError as exc:
        print(f"config error: {exc}", file=sys.stderr, flush=True)
        return 1
    return run_service(cfg)


if __name__ == "__main__":
    raise SystemExit(main())
