//! Types shared by both state machines (PROTOCOL_SPEC §6).

use zecsplit_btc::sources::{Agreement, SwapStatus};

use crate::messages::{Network, Quote};

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

/// §8 fee floor, and the ceiling a taker accepts so fees can't eat her BTC (ADR-049).
pub const FEE_RATE_RANGE: std::ops::RangeInclusive<u64> = 2..=100;

/// §8: the timelocks and confirmations per network; `None` where swaps are cut (ADR-025).
pub fn terms(network: Network) -> Option<Params> {
    let (t1, t2, n_conf_btc, n_conf_zec, redeem_margin) = match network {
        Network::Regtest => (20, 20, 1, 1, 5),
        Network::Test => (36, 36, 1, 5, 12),
        Network::MainBeta => return None,
    };
    Some(Params {
        t1,
        t2,
        n_conf_btc,
        n_conf_zec,
        redeem_margin,
    })
}

/// §8: a quote's terms are exactly its network's, with a fee rate in `FEE_RATE_RANGE`.
pub fn check_terms(q: &Quote) -> Result<(), String> {
    if terms(q.network) != Some(Params::from(q)) {
        return Err("timelocks or confirmations differ from §8".into());
    }
    if !FEE_RATE_RANGE.contains(&q.btc_fee_rate) {
        return Err(format!(
            "fee rate {} sat/vB outside {FEE_RATE_RANGE:?}",
            q.btc_fee_rate
        ));
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn quote(network: Network) -> Quote {
        let p = terms(Network::Regtest).unwrap();
        Quote {
            protocol_version: 1,
            swap_id: [7; 32],
            network,
            zec_amount: 100_000_000,
            btc_amount: 150_000,
            btc_fee_rate: 2,
            t1: p.t1,
            t2: p.t2,
            n_conf_btc: p.n_conf_btc,
            n_conf_zec: p.n_conf_zec,
            redeem_margin: p.redeem_margin,
            expires_at: u64::MAX,
        }
    }

    #[test]
    fn spec_8_taker_accepts_only_the_networks_terms() {
        assert_eq!(check_terms(&quote(Network::Regtest)), Ok(()));
        assert!(check_terms(&quote(Network::Test)).is_err());
        assert!(check_terms(&quote(Network::MainBeta)).is_err());
        let mut long = quote(Network::Regtest);
        long.t1 = u16::MAX;
        assert!(check_terms(&long).is_err());
        let mut thin = quote(Network::Regtest);
        thin.redeem_margin = 0;
        assert!(check_terms(&thin).is_err());
        for fee in [0, 1, 101, u64::MAX] {
            let mut q = quote(Network::Regtest);
            q.btc_fee_rate = fee;
            assert!(check_terms(&q).is_err(), "{fee}");
        }
    }
}
