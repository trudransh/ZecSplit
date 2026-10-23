//! Types shared by both state machines (PROTOCOL_SPEC §6).

use zecsplit_btc::sources::{Agreement, SwapStatus};

use crate::messages::Quote;

/// Quote constants the machines need (§2, §8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Params {
    pub t1: u16,
    pub t2: u16,
    pub n_conf_btc: u16,
    pub n_conf_zec: u16,
    pub redeem_margin: u16,
}

/// A Bitcoin transaction as the shell sees it.
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
    Pending,
    Mempool,
    Confirmed { confirmations: u32 },
    NothingToSweep,
}

/// The ZEC lock as Bob's own Zebra node sees it (§4.3-R1/R2, ADR-009).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZecLock {
    Unseen,
    WrongAmount,
    Correct { confirmations: u32 },
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
    pub zec_lock: ZecLock,
}

/// Terminal results (§6.1, §6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Outcome {
    Swapped,
    Refunded,
    Punished,
}

/// Why a swap stopped before any funds were at risk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AbortReason {
    PeerMisbehaved,
    NoLock,
}

/// Something the UI must tell the user (UX_SPEC §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alert {
    BtcSourcesDisagree,
    MarginTooSmall,
    PeerMisbehavedAfterLock,
}

impl From<&Quote> for Params {
    fn from(q: &Quote) -> Self {
        Params {
            t1: q.t1,
            t2: q.t2,
            n_conf_btc: q.n_conf_btc,
            n_conf_zec: q.n_conf_zec,
            redeem_margin: q.redeem_margin,
        }
    }
}

impl From<Agreement> for TxStatus {
    fn from(a: Agreement) -> Self {
        match a {
            Agreement::Unseen => TxStatus::Unseen,
            Agreement::Mempool => TxStatus::Mempool,
            Agreement::Confirmed { height } => TxStatus::Confirmed { height },
            Agreement::Disagree => TxStatus::SourcesDisagree,
        }
    }
}

impl ChainView {
    /// Nothing seen yet at `btc_tip`.
    pub fn at_tip(btc_tip: u32) -> Self {
        ChainView {
            btc_tip,
            lock: TxStatus::Unseen,
            redeem: TxStatus::Unseen,
            cancel: TxStatus::Unseen,
            refund: TxStatus::Unseen,
            punish: TxStatus::Unseen,
            zec_sweep: SweepStatus::Pending,
            zec_lock: ZecLock::Unseen,
        }
    }
}

impl From<SwapStatus> for ChainView {
    fn from(s: SwapStatus) -> Self {
        ChainView {
            lock: s.lock.into(),
            redeem: s.redeem.into(),
            cancel: s.cancel.into(),
            refund: s.refund.into(),
            punish: s.punish.into(),
            ..ChainView::at_tip(s.tip)
        }
    }
}

#[cfg(test)]
impl ChainView {
    pub(crate) fn with_lock(mut self, s: TxStatus) -> Self {
        self.lock = s;
        self
    }

    pub(crate) fn with_redeem(mut self, s: TxStatus) -> Self {
        self.redeem = s;
        self
    }

    pub(crate) fn with_zec(mut self, z: ZecLock) -> Self {
        self.zec_lock = z;
        self
    }
}
