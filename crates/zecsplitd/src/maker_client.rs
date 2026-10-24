//! The engine's HTTP client for a maker (INTERFACES §2).

use std::time::Duration;

use async_trait::async_trait;
use reqwest::StatusCode;
use zecsplit_protocol::messages::{
    Envelope, Message, Network, PROTOCOL_VERSION, QuoteRequest, SignedQuote, decode, encode,
};

use crate::runner::Maker;

/// What `GET /v1/info` says; the key is only trusted once it matches the pinned one.
pub struct MakerInfo {
    pub maker_key: [u8; 32],
    pub min_zec: u64,
    pub max_zec: u64,
}

/// A maker reached at `base`, e.g. `http://127.0.0.1:7781`.
pub struct HttpMaker {
    http: reqwest::Client,
    base: String,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

impl HttpMaker {
    pub fn new(base: &str) -> Result<Self, String> {
        Ok(HttpMaker {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .map_err(err)?,
            base: base.trim_end_matches('/').to_string(),
        })
    }

    async fn exchange(&self, req: reqwest::RequestBuilder) -> Result<Option<Envelope>, String> {
        let res = req.send().await.map_err(err)?;
        match res.status() {
            StatusCode::NO_CONTENT => Ok(None),
            StatusCode::OK => {
                let env = decode(&res.bytes().await.map_err(err)?).map_err(err)?;
                match env.message {
                    Message::Error(e) => Err(format!("maker: {}: {}", e.code, e.message)),
                    _ => Ok(Some(env)),
                }
            }
            status => Err(format!(
                "maker: http {status}: {}",
                res.text().await.map_err(err)?
            )),
        }
    }

    /// `GET /v1/info`: the maker's identity key, to pin before trusting its quotes (§2-R1).
    pub async fn identity_key(&self) -> Result<[u8; 32], String> {
        Ok(self.info().await?.maker_key)
    }

    /// `GET /v1/info`.
    pub async fn info(&self) -> Result<MakerInfo, String> {
        let info: serde_json::Value = self
            .http
            .get(format!("{}/v1/info", self.base))
            .send()
            .await
            .map_err(err)?
            .json()
            .await
            .map_err(err)?;
        let mut maker_key = [0; 32];
        hex::decode_to_slice(
            info["maker_key"].as_str().ok_or("info: no maker_key")?,
            &mut maker_key,
        )
        .map_err(err)?;
        Ok(MakerInfo {
            maker_key,
            min_zec: info["min_zec"].as_u64().ok_or("info: no min_zec")?,
            max_zec: info["max_zec"].as_u64().ok_or("info: no max_zec")?,
        })
    }

    /// `POST /v1/quote`: a signed quote, still to be verified against the pinned key (§2-R2).
    pub async fn quote(&self, zec_amount: u64, network: Network) -> Result<SignedQuote, String> {
        let env = Envelope {
            v: PROTOCOL_VERSION,
            swap_id: [0; 32],
            message: Message::QuoteRequest(QuoteRequest {
                zec_amount,
                network,
            }),
        };
        let req = self
            .http
            .post(format!("{}/v1/quote", self.base))
            .body(encode(&env).map_err(err)?);
        match self.exchange(req).await? {
            Some(Envelope {
                message: Message::Quote(q),
                ..
            }) => Ok(q),
            _ => Err("maker sent no quote".into()),
        }
    }
}

#[async_trait]
impl Maker for HttpMaker {
    async fn send(&self, msg: &Envelope) -> Result<Option<Envelope>, String> {
        let url = format!("{}/v1/swaps/{}/msg", self.base, hex::encode(msg.swap_id));
        let body = encode(msg).map_err(err)?;
        self.exchange(self.http.post(url).body(body)).await
    }

    async fn next(&self, swap_id: &[u8; 32]) -> Result<Option<Envelope>, String> {
        let url = format!("{}/v1/swaps/{}/next", self.base, hex::encode(swap_id));
        self.exchange(self.http.get(url)).await
    }
}
