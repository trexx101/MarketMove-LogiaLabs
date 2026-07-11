"""Tests for ``inference.config.InferenceConfig``."""
from __future__ import annotations

import os
from pathlib import Path

import pytest

from inference.config import InferenceConfig


INFERENCE_ENV_VARS = (
    "ZMQ_ENDPOINT",
    "ZMQ_BIND",
    "MODEL_PATH",
    "NORM_STATS_PATH",
)


@pytest.fixture
def clean_env(monkeypatch: pytest.MonkeyPatch) -> None:
    for key in INFERENCE_ENV_VARS:
        monkeypatch.delenv(key, raising=False)


def test_defaults_when_env_unset(clean_env: None) -> None:
    cfg = InferenceConfig.from_env()
    assert cfg.zmq_endpoint == "tcp://127.0.0.1:5555"
    assert cfg.zmq_bind == "tcp://*:5555"
    assert cfg.model_path == Path("/models/model.pt").resolve()
    assert cfg.norm_stats_path == Path("/models/norm_stats.json").resolve()
    assert isinstance(cfg.model_path, Path)
    assert isinstance(cfg.norm_stats_path, Path)


def test_summary_does_not_leak_secrets(clean_env: None) -> None:
    cfg = InferenceConfig.from_env()
    s = cfg.summary()
    assert "model_path" in s
    assert "norm_stats_path" in s
    assert "zmq_endpoint" in s
    assert "zmq_bind" in s
    assert "KRAKEN" not in s.upper()


def test_model_path_resolved_to_absolute(clean_env: None, tmp_path: Path) -> None:
    rel = tmp_path / "rel_model.pt"
    rel.touch()
    os.environ["MODEL_PATH"] = str(rel)
    cfg = InferenceConfig.from_env()
    assert cfg.model_path.is_absolute()
    assert cfg.model_path == rel.resolve()


def test_tilde_path_expanded(clean_env: None, monkeypatch: pytest.MonkeyPatch) -> None:
    home = Path(os.environ["HOME"])
    expected_model = home / "fake_model.pt"
    monkeypatch.setenv("MODEL_PATH", "~/fake_model.pt")
    monkeypatch.setenv("NORM_STATS_PATH", "~/fake_norm.json")
    cfg = InferenceConfig.from_env()
    assert cfg.model_path == expected_model.resolve()
    assert cfg.norm_stats_path == (home / "fake_norm.json").resolve()


def test_empty_string_uses_default(clean_env: None) -> None:
    os.environ["ZMQ_ENDPOINT"] = ""
    cfg = InferenceConfig.from_env()
    assert cfg.zmq_endpoint == "tcp://127.0.0.1:5555"


def test_custom_endpoint(clean_env: None) -> None:
    os.environ["ZMQ_ENDPOINT"] = "tcp://inference:5555"
    os.environ["ZMQ_BIND"] = "tcp://0.0.0.0:5555"
    cfg = InferenceConfig.from_env()
    assert cfg.zmq_endpoint == "tcp://inference:5555"
    assert cfg.zmq_bind == "tcp://0.0.0.0:5555"


def test_require_artifacts_missing(tmp_path: Path, clean_env: None) -> None:
    os.environ["MODEL_PATH"] = str(tmp_path / "missing_model.pt")
    os.environ["NORM_STATS_PATH"] = str(tmp_path / "missing_norm.json")
    cfg = InferenceConfig.from_env()
    with pytest.raises(FileNotFoundError) as exc:
        cfg.require_artifacts()
    msg = str(exc.value)
    assert "missing_model.pt" in msg
    assert "missing_norm.json" in msg
    assert "/models/" in msg


def test_require_artifacts_present(tmp_path: Path, clean_env: None) -> None:
    m = tmp_path / "model.pt"
    n = tmp_path / "norm.json"
    m.touch()
    n.touch()
    os.environ["MODEL_PATH"] = str(m)
    os.environ["NORM_STATS_PATH"] = str(n)
    cfg = InferenceConfig.from_env()
    cfg.require_artifacts()


def test_config_is_immutable(clean_env: None) -> None:
    cfg = InferenceConfig.from_env()
    with pytest.raises(Exception):
        cfg.zmq_endpoint = "tcp://other:5555"  # type: ignore[misc]


def test_engine_main_fails_when_artifacts_missing(
    clean_env: None, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    from inference.inference_engine import main

    os.environ["MODEL_PATH"] = str(tmp_path / "missing_model.pt")
    os.environ["NORM_STATS_PATH"] = str(tmp_path / "missing_norm.json")
    rc = main()
    assert rc == 1
    captured = capsys.readouterr()
    assert "config error" in captured.err
    assert "missing_model.pt" in captured.err


def test_engine_main_succeeds_when_artifacts_present(
    clean_env: None, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    from inference.inference_engine import main

    m = tmp_path / "model.pt"
    n = tmp_path / "norm.json"
    m.touch()
    n.touch()
    os.environ["MODEL_PATH"] = str(m)
    os.environ["NORM_STATS_PATH"] = str(n)
    rc = main()
    assert rc == 0
    captured = capsys.readouterr()
    assert "inference configured" in captured.out
    assert "placeholder" in captured.out
