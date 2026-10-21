//! Key shares (cross-group DLEq), commit-reveal, proofs of knowledge, sigma rule, transcript, secret newtypes. PROTOCOL_SPEC §3.1–3.3.

pub mod dleq;
mod hash;
mod secret;
mod shared_key;

pub use secret::{Ask, BtcSecret, Share};
pub use shared_key::{KeyError, SharePub, SharedKey, Sigma, derive_nk_rivk, shared_ak, shared_ask};
