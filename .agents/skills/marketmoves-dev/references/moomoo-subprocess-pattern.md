# Moomoo Python Subprocess Pattern

The engine shells out to Python scripts in `.agents/skills/moomooapi/scripts/`
rather than calling the OpenD TCP gateway directly — there is no Rust SDK for
OpenD, and the official `moomoo` Python package is the only maintained client.

## Pattern (mirrors `exec/moomoo.rs`)

```rust
use tokio::process::Command;
use std::path::PathBuf;
use std::process::Stdio;

fn script_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .join(format!(".agents/skills/moomooapi/scripts/quote/{name}"))
}

pub async fn backfill(pool: &DbPool, symbol: &str, ...) -> Result<usize> {
    let code = to_moomoo_code(symbol); // QQQ → US.QQQ
    let script = script_path("get_kline.py");

    let output = Command::new("python3")
        .arg(&script)
        .arg(&code)
        .arg("--ktype").arg("1d")
        .arg("--num").arg(count.to_string())
        .arg("--rehab").arg("forward")
        .arg("--json")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("spawning get_kline.py")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("get_kline.py failed (code={code}): {stderr}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let resp: KlineResponse = serde_json::from_str(&stdout)
        .with_context(|| format!("decode get_kline.py JSON for {code}"))?;

    // Parse rows → EquityCandle → upsert
}
```

## Python SDK installation

The scripts in `.agents/skills/moomooapi/scripts/` import `moomoo` (the
official Moomoo OpenAPI Python SDK). It must be installed in the Python
environment that `python3` resolves to (both on the VPS host and inside
the Docker container):

```bash
# On VPS host (if running scripts directly)
pip install moomoo-api    # or: uv pip install moomoo-api

# Minimum version: 10.4.6408
# Check: python3 -c "import moomoo; print(moomoo.__version__)"
```

Without this package, `get_kline.py` and `get_snapshot.py` fail at import
time with `ModuleNotFoundError: No module named 'moomoo'`. The Rust wrapper
treats any non-zero exit as an error and propagates it to the caller, which
triggers the Yahoo fallback.

The engine Dockerfile should include `python3-pip` and `moomoo-api` in its
`RUN pip install` step. If the Dockerfile doesn't include it, the scripts
silently fail inside the container and Yahoo fallback activates without
any indication that Moomoo was even tried. Verify with:
```bash
docker exec mmn-engine python3 -c "import moomoo"  # should exit 0
```

## Key points
- **Scripts are in `.agents/skills/moomooapi/scripts/quote/`** — `get_kline.py` for OHLCV, `get_snapshot.py` for live quotes
- **`python3` must be available** in the Docker image (the engine Dockerfile already installs it)
- **`--json` flag** must be passed — the scripts default to table output without it
- **Symbol conversion**: `QQQ` → `US.QQQ`. Already-prefixed codes (`US.AAPL`) pass through unchanged
- **Error handling**: non-zero exit → `anyhow::Error` returned to caller. Caller logs + surfaces via telemetry

## OpenD availability check

Before shelling out, check if the OpenD TCP gateway is reachable:

```rust
pub async fn is_available() -> bool {
    let host = std::env::var("FUTU_OPEND_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let port: u16 = std::env::var("FUTU_OPEND_PORT")
        .unwrap_or_else(|_| "11111".into())
        .parse()
        .unwrap_or(11111);
    tokio::net::TcpStream::connect((host.as_str(), port)).await.is_ok()
}
```

This is fast (~1ms on localhost, ~50ms remote) and avoids the 5-15s timeout
from spawning a Python process that hangs on OpenD connect.

## Docker networking note

Inside Docker containers, `127.0.0.1` points to the container itself, not
the host. Set `FUTU_OPEND_HOST=host.docker.internal` when running OpenD on
the VPS host and the engine in a container. Docker Desktop/Mac supports this
natively; on Linux, add `extra_hosts: ["host.docker.internal:host-gateway"]`
to docker-compose.yml's engine service.

## Time parsing

Moomoo daily candles return `time_key` as `"2024-01-02 00:00:00"` (US Eastern
midnight). Parse as UTC — the ±5h offset doesn't matter for daily bar alignment:

```rust
fn parse_time_key(time_key: &str) -> Result<i64> {
    chrono::NaiveDateTime::parse_from_str(time_key, "%Y-%m-%d %H:%M:%S")
        .map(|dt| dt.and_utc().timestamp())
        .with_context(|| format!("parse time_key: {}", time_key))
}
```
