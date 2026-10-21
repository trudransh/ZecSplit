//! Types shared by both state machines (PROTOCOL_SPEC §6).
//!
//! The machines are pure ("sans-IO"): `step(state, input) -> (state, actions)`. The engine
//! shell verifies every message and chain fact before it becomes an input, performs the
//! actions, and persists the returned state **before** performing them (write-ahead,
//! §6.3-R1). Every action is idempotent, so a crash between persist and act is repaired by
//! replaying the step.
//!
//! Chain input is level-triggered: each tick passes a full [`ChainView`], not "what
//! changed". Resuming after downtime (§6.3-R2) is just stepping with the current view, and
//! a machine fast-forwards through every deadline that passed while it was offline.

/// Quote constants the machines need (§2, §8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Params {
    pub t1: u16,
    pub t2: u16,
    pub n_conf_btc: u16,
    pub n_conf_zec: u16,
    pub redeem_margin: u16,
}

/// A Bitcoin transaction as the shell sees it. For the taker this is the agreement of two
/// independent sources (§4.2-R1); `SourcesDisagree` is never treated as confirmed (T3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxStatus {
    Unseen,
    Mempool,
    Confirmed { height: u32 },
    SourcesDisagree,
}

impl TxStatus {
    /// The confirmation height once `tip` gives at least `need` confirmations.
    pub fn confirmed_height(self, tip: u32, need: u16) -> Option<u32> {
        match self {
            TxStatus::Confirmed { height } if tip >= height => {
                let confirmations = u64::from(tip) - u64::from(height) + 1;
                (confirmations >= u64::from(need)).then_some(height)
            }
            TxStatus::Confirmed { .. }
            | TxStatus::Unseen
            | TxStatus::Mempool
            | TxStatus::SourcesDisagree => None,
        }
    }

    /// Seen at any depth, confirmed or not.
    pub fn seen(self) -> bool {
        matches!(self, TxStatus::Mempool | TxStatus::Confirmed { .. })
    }
}

/// A Zcash sweep of the lock notes (§4.5-R3 for Bob, §5.3-R3 for Alice).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SweepStatus {
    /// Not built or not seen yet.
    Pending,
    Mempool,
    Confirmed {
        confirmations: u32,
    },
    /// The lock address holds no notes (for example, Alice never paid).
    NothingToSweep,
}

/// One snapshot of everything on chain that matters to a swap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChainView {
    pub btc_tip: u32,
    pub lock: TxStatus,
    pub redeem: TxStatus,
    pub cancel: TxStatus,
    pub refund: TxStatus,
    pub punish: TxStatus,
    pub zec_sweep: SweepStatus,
}

/// Terminal results (§6.1, §6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Alice redeemed the BTC; Bob swept the ZEC.
    Swapped,
    /// Cancel, then refund: each party got its own asset back.
    Refunded,
    /// Bob never refunded: Alice took the BTC, the ZEC stays locked.
    Punished,
}

/// Why a swap stopped before any funds were at risk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbortReason {
    /// A reveal, a signature or the lock template failed verification (§3.2-R3, §3.5-R4).
    PeerMisbehaved,
    /// The maker never locked BTC in time (S-02).
    NoLock,
}

/// Something the UI must tell the user (UX_SPEC §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alert {
    BtcSourcesDisagree,
    MarginTooSmall,
    PeerMisbehavedAfterLock,
}
