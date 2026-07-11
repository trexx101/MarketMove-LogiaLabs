"""MarketMarkovNet inference microservice (ZMQ REP).

Feature 04 implements the full REQ/REP loop. This Feature 03 file
loads and validates the inference-side configuration so the service
fails fast on misconfiguration before any model loading.
"""
from __future__ import annotations

import sys

from .config import InferenceConfig


def main() -> int:
    cfg = InferenceConfig.from_env()
    print(f"inference configured: {cfg.summary()}", flush=True)
    try:
        cfg.require_artifacts()
    except FileNotFoundError as e:
        print(f"config error: {e}", file=sys.stderr, flush=True)
        return 1
    print("inference engine placeholder — see Feature 04", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
