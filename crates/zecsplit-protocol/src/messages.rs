//! Wire messages between taker and maker (PROTOCOL_SPEC §7, INTERFACES §2): JSON over HTTPS,
//! bytes as lowercase hex.

use serde::{Deserialize, Serialize};
use zecsplit_crypto::{MakerSecret, Reveal, SharePub, sign_quote, verify_quote};

/// §7-R1: every message carries this, and anything else is rejected.
pub const PROTOCOL_VERSION: u16 = 1;

/// PROTOCOL_SPEC §2 `network`: `test` is Zcash testnet plus Bitcoin signet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Network {
    Regtest,
    Test,
    MainBeta,
}

/// `{ "v": 1, "swap_id": "<hex32>", "type": "<Type>", "body": {...} }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    pub v: u16,
    #[serde(with = "hex::serde")]
    pub swap_id: [u8; 32],
    #[serde(flatten)]
    pub message: Message,
}

/// §7 sequence, in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "body")]
pub enum Message {
    QuoteRequest(QuoteRequest),
    Quote(SignedQuote),
    Commit(CommitBody),
    Reveal(Box<RevealBody>),
    LockTemplate(LockTemplate),
    SetupSigsAlice(SetupSigsAlice),
    SetupSigsBob(SetupSigsBob),
    LockBroadcast(LockBroadcast),
    RedeemEncSig(RedeemEncSig),
    Status(StatusBody),
    Error(ErrorBody),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuoteRequest {
    pub zec_amount: u64,
    pub network: Network,
}

/// The quote fields of §2, exactly what the maker signs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quote {
    pub protocol_version: u16,
    #[serde(with = "hex::serde")]
    pub swap_id: [u8; 32],
    pub network: Network,
    pub zec_amount: u64,
    pub btc_amount: u64,
    pub btc_fee_rate: u64,
    pub t1: u16,
    pub t2: u16,
    pub n_conf_btc: u16,
    pub n_conf_zec: u16,
    pub redeem_margin: u16,
    pub expires_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedQuote {
    #[serde(flatten)]
    pub quote: Quote,
    #[serde(with = "hex::serde")]
    pub maker_sig: [u8; 64],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitBody {
    #[serde(with = "hex::serde")]
    pub commitment: [u8; 32],
}

/// §3.2-R1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevealBody(#[serde(with = "RevealDef")] pub Reveal);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockTemplate {
    pub psbt_base64: String,
    pub lock_vout: u32,
    pub v_lock_sats: u64,
    pub refund_addr: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetupSigsAlice {
    #[serde(with = "hex::serde")]
    pub sig_cancel: Vec<u8>,
    #[serde(with = "hex::serde")]
    pub esig_refund: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetupSigsBob {
    #[serde(with = "hex::serde")]
    pub sig_cancel: Vec<u8>,
    #[serde(with = "hex::serde")]
    pub sig_punish: Vec<u8>,
}

/// Informational: Alice verifies `tx_lock` on chain herself (§4.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockBroadcast {
    pub txid: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedeemEncSig {
    #[serde(with = "hex::serde")]
    pub esig_redeem: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusBody {
    pub state: String,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "SharePub")]
struct SharePubDef {
    #[serde(with = "hex::serde")]
    pallas: [u8; 32],
    #[serde(with = "hex::serde")]
    secp: [u8; 33],
}

/// §3.2-R1, the same fields as `zecsplit_crypto::Reveal`.
#[derive(Serialize, Deserialize)]
#[serde(remote = "Reveal")]
struct RevealDef {
    #[serde(with = "SharePubDef")]
    share_pub: SharePub,
    #[serde(with = "hex::serde")]
    btc_key: [u8; 33],
    #[serde(with = "hex::serde")]
    nonce: [u8; 32],
    #[serde(with = "hex::serde")]
    dleq: Vec<u8>,
    #[serde(with = "hex::serde")]
    pok_p: [u8; 64],
    #[serde(with = "hex::serde")]
    pok_s: [u8; 64],
    #[serde(with = "hex::serde")]
    pok_k: [u8; 64],
    payout_addr: String,
}

#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error("unsupported protocol version {0}")]
    UnknownVersion(u16),
    #[error("malformed message: {0}")]
    Malformed(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QuoteError {
    #[error("quote is for protocol version {0}")]
    UnsupportedVersion(u16),
    #[error("quote signature is not from the pinned maker key")]
    BadSignature,
    #[error("quote expired")]
    Expired,
    #[error("could not sign or encode the quote: {0}")]
    Encoding(String),
}

pub fn encode(envelope: &Envelope) -> Result<Vec<u8>, WireError> {
    Ok(serde_json::to_vec(envelope)?)
}

/// §7-R1: the version is checked before anything else is parsed, since a future version may
/// change every body.
pub fn decode(bytes: &[u8]) -> Result<Envelope, WireError> {
    #[derive(Deserialize)]
    struct VersionOnly {
        v: u16,
    }
    let VersionOnly { v } = serde_json::from_slice(bytes)?;
    if v != PROTOCOL_VERSION {
        return Err(WireError::UnknownVersion(v));
    }
    Ok(serde_json::from_slice(bytes)?)
}

impl Quote {
    /// The signed form: compact JSON with keys sorted.
    pub fn canonical_json(&self) -> Result<Vec<u8>, QuoteError> {
        let value = serde_json::to_value(self).map_err(|e| QuoteError::Encoding(e.to_string()))?;
        serde_json::to_vec(&value).map_err(|e| QuoteError::Encoding(e.to_string()))
    }

    /// Maker side (§2-R2).
    pub fn sign(self, maker: &MakerSecret) -> Result<SignedQuote, QuoteError> {
        let maker_sig = sign_quote(maker, &self.canonical_json()?)
            .map_err(|e| QuoteError::Encoding(e.to_string()))?;
        Ok(SignedQuote {
            quote: self,
            maker_sig,
        })
    }
}

impl SignedQuote {
    /// Taker side (§2-R2): Alice MUST check the signature under her pinned maker key and that
    /// the quote hasn't expired before going on (T23, ADV-14).
    pub fn verify(&self, pinned_maker_key: &[u8; 32], now_unix: u64) -> Result<&Quote, QuoteError> {
        if self.quote.protocol_version != PROTOCOL_VERSION {
            return Err(QuoteError::UnsupportedVersion(self.quote.protocol_version));
        }
        verify_quote(
            pinned_maker_key,
            &self.quote.canonical_json()?,
            &self.maker_sig,
        )
        .map_err(|_| QuoteError::BadSignature)?;
        if now_unix >= self.quote.expires_at {
            return Err(QuoteError::Expired);
        }
        Ok(&self.quote)
    }
}

#[cfg(test)]
mod tests {
    use zecsplit_crypto::rand_core::OsRng;
    use zecsplit_crypto::{Role, generate, maker_public_key, verify_reveal};

    use super::*;

    fn quote() -> Quote {
        Quote {
            protocol_version: 1,
            swap_id: [0xab; 32],
            network: Network::Regtest,
            zec_amount: 100_000_000,
            btc_amount: 150_000,
            btc_fee_rate: 2,
            t1: 20,
            t2: 20,
            n_conf_btc: 1,
            n_conf_zec: 1,
            redeem_margin: 5,
            expires_at: 1_800_000_000,
        }
    }

    fn maker() -> MakerSecret {
        MakerSecret::from_bytes([5; 32])
    }

    #[test]
    fn spec_7_r1_rejects_unknown_versions() {
        let msg = r#"{"v":2,"swap_id":"00","type":"Status","body":{"state":"x","note":""}}"#;
        assert!(matches!(
            decode(msg.as_bytes()),
            Err(WireError::UnknownVersion(2))
        ));
    }

    #[test]
    fn interfaces_2_envelope_has_the_documented_shape() {
        let env = Envelope {
            v: 1,
            swap_id: [1; 32],
            message: Message::Commit(CommitBody {
                commitment: [2; 32],
            }),
        };
        let json = String::from_utf8(encode(&env).unwrap()).unwrap();
        assert_eq!(
            json,
            format!(
                r#"{{"v":1,"swap_id":"{}","type":"Commit","body":{{"commitment":"{}"}}}}"#,
                "01".repeat(32),
                "02".repeat(32)
            )
        );
        assert_eq!(decode(json.as_bytes()).unwrap(), env);
    }

    /// The bytes every maker must sign for this quote.
    #[test]
    fn spec_2_r2_canonical_quote_json_is_sorted_and_compact() {
        let json = String::from_utf8(quote().canonical_json().unwrap()).unwrap();
        assert_eq!(
            json,
            format!(
                r#"{{"btc_amount":150000,"btc_fee_rate":2,"expires_at":1800000000,"n_conf_btc":1,"n_conf_zec":1,"network":"regtest","protocol_version":1,"redeem_margin":5,"swap_id":"{}","t1":20,"t2":20,"zec_amount":100000000}}"#,
                "ab".repeat(32)
            )
        );
    }

    #[test]
    fn spec_2_r2_signed_quote_survives_the_wire_in_any_key_order() {
        let signed = quote().sign(&maker()).unwrap();
        let pinned = maker_public_key(&maker()).unwrap();
        let mut value = serde_json::to_value(&signed).unwrap();
        let reversed: serde_json::Map<String, serde_json::Value> = value
            .as_object_mut()
            .unwrap()
            .clone()
            .into_iter()
            .rev()
            .collect();
        let parsed: SignedQuote = serde_json::from_value(reversed.into()).unwrap();
        assert_eq!(parsed.verify(&pinned, 1_700_000_000), Ok(&quote()));
    }

    #[test]
    fn adv_14_quote_from_another_maker_or_edited_is_rejected() {
        let pinned = maker_public_key(&maker()).unwrap();
        let forged = quote().sign(&MakerSecret::from_bytes([6; 32])).unwrap();
        assert_eq!(forged.verify(&pinned, 0), Err(QuoteError::BadSignature));

        let mut edited = quote().sign(&maker()).unwrap();
        edited.quote.btc_amount += 1;
        assert_eq!(edited.verify(&pinned, 0), Err(QuoteError::BadSignature));
    }

    #[test]
    fn spec_2_r2_rejects_expired_quotes() {
        let pinned = maker_public_key(&maker()).unwrap();
        let signed = quote().sign(&maker()).unwrap();
        assert_eq!(
            signed.verify(&pinned, 1_800_000_000),
            Err(QuoteError::Expired)
        );
    }

    /// A real reveal (DLEq, three PoKs) crosses the wire and still verifies.
    #[test]
    fn spec_7_reveal_round_trips_and_still_verifies() {
        let swap = [3; 32];
        let alice = generate(&mut OsRng, &swap, Role::Alice).unwrap();
        let bob = generate(&mut OsRng, &swap, Role::Bob).unwrap();
        let (c_a, c_b) = (
            alice.commitment(&swap, Role::Alice),
            bob.commitment(&swap, Role::Bob),
        );
        let reveal = bob
            .reveal(&swap, Role::Bob, &c_a, &c_b, "bcrt1q".into())
            .unwrap();

        let bytes = encode(&Envelope {
            v: 1,
            swap_id: swap,
            message: Message::Reveal(Box::new(RevealBody(reveal.clone()))),
        })
        .unwrap();
        let Message::Reveal(body) = decode(&bytes).unwrap().message else {
            panic!("expected a Reveal");
        };
        let RevealBody(received) = *body;
        assert_eq!(received, reveal);
        assert_eq!(
            verify_reveal(
                &swap,
                Role::Bob,
                &c_b,
                &received,
                &c_a,
                &c_b,
                &alice.share_pub
            ),
            Ok(())
        );
    }
}
