# totp-rs Integration — Runtime Mode Toggle Reference

## Why this file
`totp-rs` v5 has non-obvious feature flags that gate method availability. Getting the wrong combination produces confusing compile errors (`no method named get_url`, `this function takes 5 arguments but 7`, etc.). This file captures the exact `Cargo.toml` line + the working API surface that the runtime mode toggle uses.

## Cargo.toml (correct)

```toml
# workspace Cargo.toml
totp-rs = { version = "5", features = ["gen_secret", "otpauth"] }

# engine/Cargo.toml
totp-rs = { workspace = true }
```

Both features are required:
- `gen_secret` — unlocks `Secret::generate_secret()` and `Secret::default()` (used at startup to mint a fresh secret if `TOTP_SECRET` env is unset).
- `otpauth` — unlocks the 7-arg `TOTP::new(algo, digits, skew, step, bytes, issuer, label)`, the `issuer` / `account_name` fields, and `TOTP::get_url()`. Without this, `new` is 5-arg and `get_url` doesn't exist.

There's **no `std` feature** — base crate is std-only. Don't try to enable it (errors with "totp-rs does not have that feature").

## API surface used by `engine/src/totp.rs`

```rust
use totp_rs::{Algorithm, Secret, TOTP};

// Constants
const DEFAULT_DIGITS: usize = 6;
const DEFAULT_STEP_SECS: u64 = 30;
const DEFAULT_SKEW: u8 = 1;  // ±1 step on each side — already covers boundary drift
const DEFAULT_ALGO: Algorithm = Algorithm::SHA1;

// Build from a base32 secret string
let bytes = Secret::Encoded(secret_b32.to_string())
    .to_bytes()
    .map_err(|e| anyhow!("invalid base32: {e}"))?;
let totp = TOTP::new(
    Algorithm::SHA1,
    6,      // digits
    1,      // skew — accept ±1 step (30s on each side)
    30,     // step seconds
    bytes,
    Some("MarketMoves".to_string()),  // issuer (otpauth)
    "control-room".to_string(),      // account label (otpauth)
)?;

// Verify a code (already covers boundary thanks to skew=1)
let ok = totp.check(code, unix_secs_now);

// Generate current code (tests + operator tooling)
let code = totp.generate_current()?;

// otpauth URL for QR scanning
let url = totp.get_url();

// Generate fresh secret (gen_secret feature)
let raw = Secret::generate_secret();           // Secret::Raw(Vec<u8>) of 20 bytes
let b32 = base32::encode(Alphabet::Rfc4648 { padding: false }, &raw.to_bytes()?);
```

## Re-encoding a generated `Secret::Raw` to base32
`Secret` implements `Display`, but `to_string()` on `Secret::Raw(...)` gives **hex**, not base32. After `Secret::generate_secret()`, the safe path is:

```rust
let secret = Secret::generate_secret();
let raw_bytes = secret.to_bytes()?;  // Vec<u8>
let encoded = base32::encode(
    base32::Alphabet::Rfc4648 { padding: false },
    &raw_bytes,
);
```

If you can't pull `base32` directly (it's a transitive dep of totp-rs), the inline encoder in `engine/src/totp.rs::base32_encode` is the workaround — 30 lines, RFC 4648 compliant, validated against the `"foobar" -> "MZXW6YTBOI"` test vector.

## Time source for `check()`
`TOTP::check(&self, token: &str, time: u64) -> bool` takes **`u64`** (not `i64`). Convert from chrono via SystemTime:

```rust
let now = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map_err(|e| anyhow!("clock before unix epoch: {e}"))?
    .as_secs();
```

## Tests that round-trip the secret
`Secret::generate_secret()` → bytes → `base32::encode` (no padding) → `Secret::Encoded(encoded_str).to_bytes()` MUST equal the original bytes. `engine/src/totp.rs::tests::base32_round_trips_with_totp_rs` exercises this end-to-end and should always pass.

## What is NOT in scope for this skill
- QR-code PNG generation (`totp-rs` `qr` feature pulls `qrcodegen-image`; skipped to keep the binary small — frontend renders the otpauth URL into a QR client-side via a JS lib).
- Steam TOTP (`steam` feature; not relevant for control-room auth).
- `serde_support` / `urlencoding` features (no need to serialize TOTP config; otpauth URL building is handled by `get_url`).

## Common compile errors and their fix
| Error | Fix |
|-------|-----|
| `no method named get_url found for struct TOTP` | Add `features = ["otpauth"]` to `totp-rs`. |
| `this function takes 5 arguments but 7 were supplied` | Same — without `otpauth`, `new` is 5-arg (no issuer / label). |
| `no method named generate_secret found in Secret` | Add `features = ["gen_secret"]`. |
| `totp-rs does not have that feature: std` | Remove `features = ["std"]` — there's no such feature. |
| `no method named trim_end_matches found for enum totp_rs::Secret` | `Secret` doesn't expose string methods; convert to `String` via Display or `to_encoded()`/`to_bytes()` first. |