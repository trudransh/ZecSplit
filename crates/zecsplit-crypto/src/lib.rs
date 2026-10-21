//! Key shares (cross-group DLEq), commit-reveal, proofs of knowledge, sigma rule, transcript, secret newtypes. PROTOCOL_SPEC §3.1–3.3.

mod secret;

pub use secret::{Ask, BtcSecret, Share};
