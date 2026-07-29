//! TOTP (RFC 6238) helper for the runtime paper/live mode toggle (Phase 3.4).
//!
//! Thin facade over `totp-rs` so callers don't need to know about `Secret`,
//! `Algorithm`, or `TOTP::new` plumbing.
//!
//! ## Secret storage
//!
//! On first run (or whenever `TOTP_SECRET` env is empty), `load_or_generate_secret`
//! will mint a fresh 160-bit base32 secret. The returned secret is the canonical
//! representation that the caller MUST persist somewhere durable (env var or
//! restricted-permission file). All subsequent invocations must reuse the same
//! secret — regenerating it locks the user out.
//!
//! ## TOTP algorithm
//!
//! SHA-1 / 6 digits / 30-second period with skew=1 (the Google Authenticator
//! default and the algorithm that all consumer authenticator apps support).
//! The skew lets `verify` accept codes entered ±1 step on either side of the
//! boundary.

use anyhow::{anyhow, Context, Result};
use totp_rs::{Algorithm, Secret, TOTP};

const DEFAULT_DIGITS: usize = 6;
const DEFAULT_STEP_SECS: u64 = 30;
/// ±1 step on each side of the current period (1 step = 30s).
const DEFAULT_SKEW: u8 = 1;
const DEFAULT_ALGO: Algorithm = Algorithm::SHA1;

/// Issuer label embedded in the otpauth URL (shown in the user's authenticator app).
pub const ISSUER: &str = "MarketMoves";

/// Account label embedded in the otpauth URL.
pub const ACCOUNT_LABEL: &str = "control-room";

/// Verify a 6-digit TOTP code against the given base32 secret.
///
/// Skew is configured at `TOTP::new` time, so `check` already accepts the
/// boundary window. We still re-validate the code's shape to fail fast on
/// gibberish input.
pub fn verify(secret_b32: &str, code: &str) -> Result<bool> {
    let trimmed = code.trim();
    if trimmed.len() != DEFAULT_DIGITS {
        return Ok(false);
    }
    if !trimmed.chars().all(|c| c.is_ascii_digit()) {
        return Ok(false);
    }
    let totp = build_totp(secret_b32)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| anyhow!("system clock before unix epoch: {e}"))?
        .as_secs();
    Ok(totp.check(trimmed, now))
}

/// Generate a fresh base32-encoded TOTP secret (160 bits).
///
/// Returns the canonical encoded form (no padding) suitable for `TOTP_SECRET`.
pub fn generate_secret() -> Result<String> {
    let secret = Secret::generate_secret();
    let raw = secret
        .to_bytes()
        .context("reading generated secret bytes")?;
    Ok(base32_encode(&raw))
}

/// Build an `otpauth://totp/...` URL that the user can scan with their
/// authenticator app.
pub fn otpauth_url(secret_b32: &str, issuer: &str, label: &str) -> Result<String> {
    let totp = build_totp_with_issuer(secret_b32, issuer, label)?;
    Ok(totp.get_url())
}

/// Load the TOTP secret from the `TOTP_SECRET` env var. If unset or empty,
/// mint a fresh secret and return it. The caller is responsible for
/// persisting the new secret before allowing any mode switches.
///
/// Returns `(secret, true)` if a fresh secret was generated.
pub fn load_or_generate_secret() -> Result<(String, bool)> {
    match std::env::var("TOTP_SECRET") {
        Ok(v) if !v.trim().is_empty() => Ok((v.trim().to_string(), false)),
        _ => Ok((generate_secret()?, true)),
    }
}

/// Current TOTP code for `secret_b32` — used by tests and operator tooling.
#[allow(dead_code)]
pub fn current_code(secret_b32: &str) -> Result<String> {
    let totp = build_totp(secret_b32)?;
    totp.generate_current()
        .context("generating current TOTP code")
}

fn build_totp(secret_b32: &str) -> Result<TOTP> {
    build_totp_with_issuer(secret_b32, ISSUER, ACCOUNT_LABEL)
}

fn build_totp_with_issuer(secret_b32: &str, issuer: &str, label: &str) -> Result<TOTP> {
    let bytes = Secret::Encoded(secret_b32.to_string())
        .to_bytes()
        .map_err(|e| anyhow!("invalid base32 secret: {e}"))
        .context("decoding TOTP secret")?;
    TOTP::new(
        DEFAULT_ALGO,
        DEFAULT_DIGITS,
        DEFAULT_SKEW,
        DEFAULT_STEP_SECS,
        bytes,
        Some(issuer.to_string()),
        label.to_string(),
    )
    .context("constructing TOTP verifier")
}

/// RFC 4648 base32 encode without padding.
fn base32_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = String::with_capacity((bytes.len() * 8 + 4) / 5);
    let mut bits: u32 = 0;
    let mut bit_count: u32 = 0;
    for &b in bytes {
        bits = (bits << 8) | b as u32;
        bit_count += 8;
        while bit_count >= 5 {
            bit_count -= 5;
            let idx = ((bits >> bit_count) & 0x1f) as usize;
            out.push(ALPHABET[idx] as char);
        }
    }
    if bit_count > 0 {
        let idx = ((bits << (5 - bit_count)) & 0x1f) as usize;
        out.push(ALPHABET[idx] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_secret_is_nonempty_base32() {
        let secret = generate_secret().unwrap();
        assert!(!secret.is_empty());
        assert!(
            secret
                .chars()
                .all(|c| matches!(c, 'A'..='Z' | '2'..='7')),
            "secret not base32: {secret}"
        );
        // 160 bits = 32 chars unpadded.
        assert!(secret.len() >= 26, "secret too short: {}", secret.len());
    }

    #[test]
    fn verify_accepts_current_code() {
        let secret = generate_secret().unwrap();
        let code = current_code(&secret).unwrap();
        assert!(verify(&secret, &code).unwrap(), "current code should verify");
    }

    #[test]
    fn verify_rejects_wrong_code() {
        let secret = generate_secret().unwrap();
        assert!(!verify(&secret, "000000").unwrap());
        assert!(!verify(&secret, "12345").unwrap()); // too short
        assert!(!verify(&secret, "abcdef").unwrap()); // non-digit
    }

    #[test]
    fn verify_rejects_empty_secret() {
        assert!(verify("", "123456").is_err());
    }

    #[test]
    fn otpauth_url_has_expected_scheme() {
        let secret = generate_secret().unwrap();
        let url = otpauth_url(&secret, ISSUER, ACCOUNT_LABEL).unwrap();
        assert!(url.starts_with("otpauth://totp/"), "url: {url}");
        assert!(url.contains("MarketMoves"));
    }

    #[test]
    fn base32_known_vector() {
        // RFC 4648 §10 test vector: "foobar" -> "MZXW6YTBOI======"
        // 6 bytes = 48 bits -> 10 base32 chars (no padding).
        let encoded = base32_encode(b"foobar");
        assert_eq!(encoded, "MZXW6YTBOI");
    }

    #[test]
    fn base32_round_trips_with_totp_rs() {
        // Whatever total value the encoder produces, totp-rs's Secret::Encoded
        // must accept it as raw bytes.
        let secret = generate_secret().unwrap();
        let totp = build_totp(&secret).unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let code = totp.generate(now);
        assert!(verify(&secret, &code).unwrap());
    }
}
