//! Key shares (cross-group DLEq), commit-reveal, proofs of knowledge, sigma rule, transcript,
//! secret newtypes.

mod dleq;
mod hash;
mod identity;
mod reveal;
mod secret;
mod shared_key;
mod transcript;

pub use identity::{QuoteSigError, maker_public_key, quote_digest, sign_quote, verify_quote};
pub use rand_core;
pub use reveal::{
    Commitment, KeyMaterial, Reveal, RevealError, Role, check_share_matches, commit, generate,
    verify_reveal,
};
pub use secret::{Ask, BtcSecret, MakerSecret, Share};
pub use shared_key::{KeyError, SharePub, SharedKey, Sigma, derive_nk_rivk, shared_ak, shared_ask};
pub use transcript::{Network, TranscriptError, TranscriptInput, transcript};
