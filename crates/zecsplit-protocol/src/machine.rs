//! Types shared by both state machines (PROTOCOL_SPEC §6).

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Swapped,
    Refunded,
    Punished,
}

/// Why a swap stopped before any funds were at risk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
