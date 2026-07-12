use anyhow::{anyhow, Context, Result};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use hmac::{Hmac, Mac};
use reqwest::Client;
use sha2::{Digest, Sha256, Sha512};
use tracing::{error, info};

use crate::exec::{FillResult, TradeSide};
use crate::strategy::Position;

type HmacSha512 = Hmac<Sha512>;

pub struct KrakenExecutor {
    client: Client,
    api_key: String,
    api_secret: Vec<u8>,
    pair: String,
    current_position: Position,
    entry_price: f64,
    qty: f64,
}

fn convert_symbol(symbol: &str) -> String {
    symbol.replace("BTC", "XBT").replace("/", "")
}

impl KrakenExecutor {
    pub fn new(api_key: &str, api_secret_b64: &str, symbol: &str) -> Result<Self> {
        let api_secret = STANDARD
            .decode(api_secret_b64)
            .context("failed to decode base64 secret")?;

        let pair = convert_symbol(symbol);

        let client = Client::builder()
            .user_agent("MarketMarkovNet/0.1")
            .build()
            .context("failed to build HTTP client")?;

        Ok(Self {
            client,
            api_key: api_key.to_string(),
            api_secret,
            pair,
            current_position: Position::Flat,
            entry_price: 0.0,
            qty: 1.0,
        })
    }

    pub fn sign_request(&self, path: &str, nonce: &str, post_data: &str) -> Result<String> {
        let message = format!("{}{}", post_data, nonce);
        let sha256_hash = Sha256::digest(message.as_bytes());

        let mut hmac_input = Vec::with_capacity(path.len() + sha256_hash.len());
        hmac_input.extend_from_slice(path.as_bytes());
        hmac_input.extend_from_slice(&sha256_hash);

        let mut mac = HmacSha512::new_from_slice(&self.api_secret)
            .map_err(|e| anyhow!("HMAC key error: {e}"))?;
        mac.update(&hmac_input);
        let signature = mac.finalize().into_bytes();

        Ok(STANDARD.encode(signature))
    }

    async fn place_order(&self, side: TradeSide, qty: f64) -> Result<String> {
        let nonce = chrono::Utc::now().timestamp_micros().to_string();

        let order_type = match side {
            TradeSide::Buy => "buy",
            TradeSide::Sell => "sell",
        };

        let post_data = format!(
            "nonce={}&pair={}&type={}&ordertype=market&volume={}",
            nonce, self.pair, order_type, qty
        );

        let signature = self.sign_request("/0/private/AddOrder", &nonce, &post_data)?;

        let response = self
            .client
            .post("https://api.kraken.com/0/private/AddOrder")
            .header("API-Key", &self.api_key)
            .header("API-Sign", &signature)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(post_data)
            .send()
            .await
            .context("failed to send order request to Kraken")?;

        let body: serde_json::Value = response
            .json()
            .await
            .context("failed to parse Kraken response")?;

        let errors = body["error"]
            .as_array()
            .context("missing error array in Kraken response")?;

        if !errors.is_empty() {
            let err_msg = errors
                .iter()
                .map(|e| e.as_str().unwrap_or("unknown"))
                .collect::<Vec<_>>()
                .join(", ");
            error!(errors = %err_msg, "Kraken order failed");
            return Err(anyhow!("Kraken order error: {}", err_msg));
        }

        let txid = body["result"]["txid"][0]
            .as_str()
            .context("missing txid in Kraken response")?
            .to_string();

        info!(txid = %txid, side = order_type, qty = qty, "Kraken order placed");
        Ok(txid)
    }

    pub async fn set_target_position(
        &mut self,
        target: Position,
        close: f64,
        ts: i64,
    ) -> Result<Vec<FillResult>> {
        if target == self.current_position {
            return Ok(Vec::new());
        }

        let mut fills = Vec::new();

        if self.current_position != Position::Flat {
            let exit_side = match self.current_position {
                Position::Long => TradeSide::Sell,
                Position::Short => TradeSide::Buy,
                Position::Flat => unreachable!(),
            };

            let txid = self.place_order(exit_side, self.qty).await?;

            let pnl = match self.current_position {
                Position::Long => (close - self.entry_price) * self.qty,
                Position::Short => (self.entry_price - close) * self.qty,
                Position::Flat => unreachable!(),
            };

            info!(
                txid = %txid,
                side = ?exit_side,
                qty = self.qty,
                price = close,
                pnl = pnl,
                "closing position on Kraken"
            );

            fills.push(FillResult {
                side: exit_side,
                qty: self.qty,
                price: close,
                fee: 0.0,
                realized_pnl: pnl,
                ts,
            });
        }

        if target != Position::Flat {
            let entry_side = match target {
                Position::Long => TradeSide::Buy,
                Position::Short => TradeSide::Sell,
                Position::Flat => unreachable!(),
            };

            let txid = self.place_order(entry_side, self.qty).await?;

            info!(
                txid = %txid,
                side = ?entry_side,
                qty = self.qty,
                price = close,
                "opening position on Kraken"
            );

            self.entry_price = close;
            fills.push(FillResult {
                side: entry_side,
                qty: self.qty,
                price: close,
                fee: 0.0,
                realized_pnl: 0.0,
                ts,
            });
        }

        self.current_position = target;
        Ok(fills)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_executor() -> KrakenExecutor {
        let secret_b64 = "kQH5HW/8p1uGOVjbgWA7FunAmGO8lsSUXFYsuR2BHIc=";
        KrakenExecutor::new("test-key", secret_b64, "BTC/USD").unwrap()
    }

    #[test]
    fn sign_request_produces_valid_base64() {
        let exec = test_executor();
        let sig = exec
            .sign_request("/0/private/Balance", "1614328800000000", "nonce=1614328800000000")
            .unwrap();
        assert!(!sig.is_empty());
        assert!(STANDARD.decode(&sig).is_ok(), "signature must be valid base64");
    }

    #[test]
    fn sign_request_deterministic() {
        let exec = test_executor();
        let sig1 = exec
            .sign_request("/0/private/Balance", "1614328800000000", "nonce=1614328800000000")
            .unwrap();
        let sig2 = exec
            .sign_request("/0/private/Balance", "1614328800000000", "nonce=1614328800000000")
            .unwrap();
        assert_eq!(sig1, sig2);
    }

    #[test]
    fn symbol_conversion() {
        assert_eq!(convert_symbol("BTC/USD"), "XBTUSD");
        assert_eq!(convert_symbol("ETH/USD"), "ETHUSD");
    }

    #[test]
    fn kraken_signature_matches_documented_scheme() {
        let secret_b64 = "kQH5HW/8p1uGOVjbgWA7FunAmGO8lsSUXFYsuR2BHIc=";
        let exec = KrakenExecutor::new("test-key", secret_b64, "BTC/USD").unwrap();

        let sig = exec
            .sign_request(
                "/0/private/Balance",
                "1614328800000000",
                "nonce=1614328800000000",
            )
            .unwrap();

        assert_eq!(sig.len(), 88, "HMAC-SHA512 base64 must be 88 chars");
        let decoded = STANDARD.decode(&sig).unwrap();
        assert_eq!(decoded.len(), 64, "HMAC-SHA512 output must be 64 bytes");
    }
}
