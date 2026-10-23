//! Saving a session as a recovery bundle and rebuilding it by replaying its inputs (§6.3-R2, §6.4).

use serde::de::DeserializeOwned;
use zecsplit_crypto::{BtcSecret, Share, check_share_matches, commit};

use super::{Core, Keys, SessionError};
use crate::messages::{Quote, RevealBody};
use crate::recovery::RecoveryBundle;

const OWN_REVEAL: &str = "own_reveal";

/// Named byte blobs as stored in a bundle.
pub(super) type Blobs = Vec<(String, Vec<u8>)>;

/// Parses one replay-log entry.
pub(super) fn parse<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, SessionError> {
    serde_json::from_slice(bytes).map_err(|e| SessionError::Key(format!("saved session: {e}")))
}

impl Core {
    /// §6.4-R1: keys, own reveal and the replay log, plus role-specific `extra` blobs.
    pub(super) fn bundle(
        &self,
        maker_endpoint: &str,
        extra: Blobs,
    ) -> Result<RecoveryBundle, SessionError> {
        let shared = self
            .shared
            .ok_or(SessionError::OutOfOrder("save only after both reveals"))?;
        let own = self
            .own_reveal
            .as_ref()
            .ok_or(SessionError::OutOfOrder("save only after revealing"))?;
        let own_json = serde_json::to_vec(&RevealBody(own.clone()))
            .map_err(|e| SessionError::Key(e.to_string()))?;
        let mut blobs = vec![(OWN_REVEAL.to_string(), own_json)];
        blobs.extend(self.log.iter().map(|(n, b)| ((*n).to_string(), b.clone())));
        blobs.extend(extra);
        Ok(RecoveryBundle {
            swap_id: self.quote.swap_id,
            role: self.role,
            quote_json: serde_json::to_string(&self.quote)
                .map_err(|e| SessionError::Key(e.to_string()))?,
            maker_endpoint: maker_endpoint.to_string(),
            share: Share::from_bytes(*self.keys.share().expose_secret()),
            btc_key: BtcSecret::from_bytes(*self.keys.btc_secret().expose_secret()),
            transcript: shared.transcript,
            blobs,
        })
    }

    /// §6.3-R2: a core with this party's keys and reveal back, before the log is replayed.
    pub(super) fn restored(b: RecoveryBundle) -> Result<(Self, Blobs), SessionError> {
        let quote: Quote = serde_json::from_str(&b.quote_json)
            .map_err(|e| SessionError::Key(format!("saved quote: {e}")))?;
        let own: RevealBody = parse(
            b.blob(OWN_REVEAL)
                .ok_or(SessionError::OutOfOrder("saved session has no reveal"))?,
        )?;
        let own = own.0;
        if quote.swap_id != b.swap_id || !check_share_matches(&b.share, &own.share_pub) {
            return Err(SessionError::Key(
                "saved session does not match its keys".into(),
            ));
        }
        let commitment = commit(&b.swap_id, b.role, &own.share_pub, &own.btc_key, &own.nonce);
        let core = Core {
            quote,
            role: b.role,
            keys: Keys::Restored {
                share: b.share,
                btc_secret: b.btc_key,
            },
            share_pub: own.share_pub,
            btc_key: own.btc_key,
            payout_addr: own.payout_addr.clone(),
            commitment,
            peer_commitment: None,
            own_reveal: Some(own),
            peer: None,
            shared: None,
            log: Vec::new(),
        };
        Ok((core, b.blobs))
    }

    /// Replays the commit-reveal entries; returns whether `name` was one of them.
    pub(super) fn replay(&mut self, name: &str, bytes: &[u8]) -> Result<bool, SessionError> {
        match name {
            "peer_commit" => self.on_commit(&parse(bytes)?)?,
            "peer_reveal" => {
                self.on_reveal(&parse(bytes)?)?;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
}
