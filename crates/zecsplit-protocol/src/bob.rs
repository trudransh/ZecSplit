//! Bob, the maker (PROTOCOL_SPEC §6.2). He has BTC and wants ZEC.
//!
//! The rules this machine exists to enforce, in order of what costs funds:
//! - `tx_lock` is broadcast only after Alice's cancel signature and refund adaptor
//!   signature are verified (§3.5-R5);
//! - `esig_B` goes out only for the exact quoted ZEC, confirmed on his own node (§4.3, §5.5);
//! - after a cancel confirms, he refunds within one block and fee-bumps (§5.3-R5), or he
//!   loses both assets to Alice's punish (T7);
//! - once Alice's redeem is visible he recovers `s_a` and sweeps; that has no deadline
//!   (§4.5-R4).

use crate::machine::{
    AbortReason, Alert, ChainView, Outcome, Params, SweepStatus, TxStatus, ZecLock,
};
use crate::timers::{cancel_valid, needs_cpfp};

/// §5.3-R5: CPFP a refund still unconfirmed one block after broadcast.
const REFUND_CPFP_AFTER: u32 = 1;

/// §6.2. `SigsExchanged` is folded into `BtcLockBroadcast` (verifying Alice's signatures
/// is exactly what permits the lock) and `ZecLockConfirmed` into `EncSigSent` (ADR-033).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BobState {
    /// Quote sent. Waiting for `C_A`.
    Quoted,
    /// `C_B` sent. Waiting for `Reveal_A`.
    Committed,
    /// `Reveal_B` and the unsigned lock template sent. Waiting for Alice's signatures.
    Revealed,
    BtcLockBroadcast,
    BtcLockConfirmed {
        h_lock: u32,
    },
    EncSigSent {
        h_lock: u32,
    },
    /// Alice's redeem is visible: `s_a` is recoverable (§4.5-R1). Sweeping the ZEC.
    RedeemSeen,
    ZecSweepBroadcast,
    CancelBroadcast {
        h_lock: u32,
    },
    Cancelled {
        h_cancel: u32,
    },
    RefundBroadcast {
        h_cancel: u32,
        at_tip: u32,
    },
    Done(Outcome),
    Aborted(AbortReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BobInput {
    /// Engine restart: re-emit this state's (idempotent) duties.
    Resume,
    PeerCommitted,
    PeerRevealVerified,
    /// Alice's `sig_cancel` and `esig_refund` verified (§3.5-R4).
    PeerSigsVerified,
    PeerMisbehaved,
    /// The shell's wall-clock timer: Alice stalled during setup.
    SetupTimedOut,
    Chain(ChainView),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BobAction {
    SendCommit,
    SendReveal,
    SendLockTemplate,
    SendSetupSigs,
    BroadcastLock,
    SendEncSig,
    /// Recover `s_a` from the redeem, check it opens `S_a^P` (§4.5-R2), sweep (§4.5-R3).
    SweepZec,
    BroadcastCancel,
    BroadcastRefund,
    CpfpRefund,
    Alert(Alert),
}

pub fn step_bob(state: BobState, input: &BobInput, p: &Params) -> (BobState, Vec<BobAction>) {
    use BobAction as A;
    use BobInput as I;
    use BobState as S;

    if let S::Done(_) | S::Aborted(_) = state {
        return (state, vec![]);
    }
    match *input {
        I::Chain(view) => on_chain(state, &view, p),
        I::Resume => (state, duties(state)),
        I::PeerMisbehaved | I::SetupTimedOut => {
            if before_lock(state) {
                let reason = match *input {
                    I::SetupTimedOut => AbortReason::NoLock,
                    _ => AbortReason::PeerMisbehaved,
                };
                (S::Aborted(reason), vec![])
            } else {
                (state, vec![A::Alert(Alert::PeerMisbehavedAfterLock)])
            }
        }
        I::PeerCommitted => match state {
            S::Quoted => (S::Committed, vec![A::SendCommit]),
            _ => (state, vec![]),
        },
        I::PeerRevealVerified => match state {
            S::Committed => (S::Revealed, vec![A::SendReveal, A::SendLockTemplate]),
            _ => (state, vec![]),
        },
        // §3.5-R5: the only way into a state that broadcasts tx_lock.
        I::PeerSigsVerified => match state {
            S::Revealed => (
                S::BtcLockBroadcast,
                vec![A::SendSetupSigs, A::BroadcastLock],
            ),
            _ => (state, vec![]),
        },
    }
}

fn before_lock(state: BobState) -> bool {
    use BobState as S;
    match state {
        S::Quoted | S::Committed | S::Revealed => true,
        S::BtcLockBroadcast
        | S::BtcLockConfirmed { .. }
        | S::EncSigSent { .. }
        | S::RedeemSeen
        | S::ZecSweepBroadcast
        | S::CancelBroadcast { .. }
        | S::Cancelled { .. }
        | S::RefundBroadcast { .. }
        | S::Done(_)
        | S::Aborted(_) => false,
    }
}

fn duties(state: BobState) -> Vec<BobAction> {
    use BobAction as A;
    use BobState as S;
    match state {
        S::Committed => vec![A::SendCommit],
        S::Revealed => vec![A::SendReveal, A::SendLockTemplate],
        S::BtcLockBroadcast => vec![A::SendSetupSigs, A::BroadcastLock],
        S::EncSigSent { .. } => vec![A::SendEncSig],
        S::RedeemSeen => vec![A::SweepZec],
        S::CancelBroadcast { .. } => vec![A::BroadcastCancel],
        S::RefundBroadcast { .. } => vec![A::BroadcastRefund],
        S::Quoted
        | S::BtcLockConfirmed { .. }
        | S::ZecSweepBroadcast
        | S::Cancelled { .. }
        | S::Done(_)
        | S::Aborted(_) => vec![],
    }
}

fn on_chain(state: BobState, v: &ChainView, p: &Params) -> (BobState, Vec<BobAction>) {
    use BobAction as A;
    use BobState as S;
    let tip = v.btc_tip;

    match state {
        S::Quoted | S::Committed | S::Revealed | S::Done(_) | S::Aborted(_) => (state, vec![]),

        S::BtcLockBroadcast => match v.lock.confirmed_height(tip, p.n_conf_btc) {
            Some(h_lock) => on_chain(S::BtcLockConfirmed { h_lock }, v, p),
            None if v.lock.seen() => (state, vec![]),
            None => (state, vec![A::BroadcastLock]),
        },

        S::BtcLockConfirmed { h_lock } => {
            if let Some(done) = lock_phase_outcome(h_lock, v, p) {
                return done;
            }
            match v.zec_lock {
                // §4.3-R2/R3: exact amount, every note unspent, enough confirmations.
                ZecLock::Correct { confirmations } if confirmations >= u32::from(p.n_conf_zec) => {
                    (S::EncSigSent { h_lock }, vec![A::SendEncSig])
                }
                ZecLock::Correct { .. } | ZecLock::Unseen | ZecLock::WrongAmount => (state, vec![]),
            }
        }

        S::EncSigSent { h_lock } => match lock_phase_outcome(h_lock, v, p) {
            Some(done) => done,
            None => (state, vec![]),
        },

        S::RedeemSeen | S::ZecSweepBroadcast => match v.zec_sweep {
            SweepStatus::Confirmed { confirmations }
                if confirmations >= u32::from(p.n_conf_zec) =>
            {
                (S::Done(Outcome::Swapped), vec![])
            }
            SweepStatus::Confirmed { .. } | SweepStatus::Mempool => (S::ZecSweepBroadcast, vec![]),
            // Alice can't move the notes (she knows only s_a), so Bob retries forever (§4.5-R4).
            SweepStatus::Pending | SweepStatus::NothingToSweep => {
                (S::RedeemSeen, vec![A::SweepZec])
            }
        },

        S::CancelBroadcast { .. } => {
            if v.redeem.seen() {
                return on_chain(S::RedeemSeen, v, p);
            }
            match v.cancel {
                TxStatus::Confirmed { height } => on_chain(S::Cancelled { h_cancel: height }, v, p),
                TxStatus::Unseen | TxStatus::SourcesDisagree => (state, vec![A::BroadcastCancel]),
                TxStatus::Mempool => (state, vec![]),
            }
        }

        // §5.3-R5: refund at once.
        S::Cancelled { h_cancel } => match refund_outcome(v, p) {
            Some(done) => done,
            None => (
                S::RefundBroadcast {
                    h_cancel,
                    at_tip: tip,
                },
                vec![A::BroadcastRefund],
            ),
        },

        S::RefundBroadcast { h_cancel, at_tip } => {
            if let Some(done) = refund_outcome(v, p) {
                return done;
            }
            match v.refund {
                TxStatus::Unseen | TxStatus::SourcesDisagree => (state, vec![A::BroadcastRefund]),
                TxStatus::Mempool if needs_cpfp(tip, at_tip, REFUND_CPFP_AFTER) => (
                    S::RefundBroadcast {
                        h_cancel,
                        at_tip: tip,
                    },
                    vec![A::CpfpRefund],
                ),
                TxStatus::Mempool | TxStatus::Confirmed { .. } => (state, vec![]),
            }
        }
    }
}

/// Facts that end the lock phase: Alice's redeem is visible, or a cancel is on chain, or
/// `t1` passed and Bob must cancel himself (§5.2, §5.3-R1).
fn lock_phase_outcome(
    h_lock: u32,
    v: &ChainView,
    p: &Params,
) -> Option<(BobState, Vec<BobAction>)> {
    // Seen is enough: the witness already carries the decrypted signature (§4.5-R1).
    if v.redeem.seen() {
        return Some(on_chain(BobState::RedeemSeen, v, p));
    }
    if let TxStatus::Confirmed { height } = v.cancel {
        return Some(on_chain(BobState::Cancelled { h_cancel: height }, v, p));
    }
    if v.cancel.seen() {
        return Some((BobState::CancelBroadcast { h_lock }, vec![]));
    }
    if cancel_valid(v.btc_tip, h_lock, p.t1) {
        return Some((
            BobState::CancelBroadcast { h_lock },
            vec![BobAction::BroadcastCancel],
        ));
    }
    None
}

fn refund_outcome(v: &ChainView, p: &Params) -> Option<(BobState, Vec<BobAction>)> {
    if v.refund.confirmed_height(v.btc_tip, p.n_conf_btc).is_some() {
        return Some((BobState::Done(Outcome::Refunded), vec![]));
    }
    if let TxStatus::Confirmed { .. } = v.punish {
        return Some((BobState::Done(Outcome::Punished), vec![]));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use BobAction as A;
    use BobInput as I;
    use BobState as S;

    const P: Params = Params {
        t1: 20,
        t2: 20,
        n_conf_btc: 1,
        n_conf_zec: 2,
        redeem_margin: 5,
    };
    const H_LOCK: u32 = 100;

    fn view(tip: u32) -> ChainView {
        ChainView {
            btc_tip: tip,
            lock: TxStatus::Confirmed { height: H_LOCK },
            redeem: TxStatus::Unseen,
            cancel: TxStatus::Unseen,
            refund: TxStatus::Unseen,
            punish: TxStatus::Unseen,
            zec_sweep: SweepStatus::Pending,
            zec_lock: ZecLock::Unseen,
        }
    }

    fn chain(state: BobState, v: ChainView) -> (BobState, Vec<BobAction>) {
        step_bob(state, &I::Chain(v), &P)
    }

    #[test]
    fn s_01_happy_path() {
        let (s, a1) = step_bob(S::Quoted, &I::PeerCommitted, &P);
        let (s, a2) = step_bob(s, &I::PeerRevealVerified, &P);
        let (s, a3) = step_bob(s, &I::PeerSigsVerified, &P);
        assert_eq!(s, S::BtcLockBroadcast);
        assert_eq!(
            [a1, a2, a3].concat(),
            [
                A::SendCommit,
                A::SendReveal,
                A::SendLockTemplate,
                A::SendSetupSigs,
                A::BroadcastLock
            ]
        );
        let (s, acts) = chain(s, view(H_LOCK));
        assert_eq!((s, acts), (S::BtcLockConfirmed { h_lock: H_LOCK }, vec![]));

        let mut v = view(H_LOCK + 2);
        v.zec_lock = ZecLock::Correct { confirmations: 2 };
        let (s, acts) = chain(s, v);
        assert_eq!(
            (s, acts),
            (S::EncSigSent { h_lock: H_LOCK }, vec![A::SendEncSig])
        );

        v.redeem = TxStatus::Mempool;
        let (s, acts) = chain(s, v);
        assert_eq!((s, acts), (S::RedeemSeen, vec![A::SweepZec]));
        v.zec_sweep = SweepStatus::Confirmed { confirmations: 2 };
        assert_eq!(chain(s, v), (S::Done(Outcome::Swapped), vec![]));
    }

    /// §3.5-R5: no input sequence reaches a `BroadcastLock` without Alice's signatures.
    #[test]
    fn spec_3_5_r5_no_lock_broadcast_before_alices_signatures() {
        for state in [S::Quoted, S::Committed, S::Revealed] {
            for input in [
                I::Resume,
                I::PeerCommitted,
                I::PeerRevealVerified,
                I::Chain(view(H_LOCK)),
            ] {
                let (_, acts) = step_bob(state, &input, &P);
                assert!(!acts.contains(&A::BroadcastLock), "{state:?} + {input:?}");
            }
        }
    }

    #[test]
    fn spec_4_3_r2_enc_sig_only_for_the_exact_amount_with_confirmations() {
        let s = S::BtcLockConfirmed { h_lock: H_LOCK };
        let mut v = view(H_LOCK + 2);
        v.zec_lock = ZecLock::Correct { confirmations: 1 };
        assert_eq!(chain(s, v), (s, vec![]));
        // S-06 / §5.5: wrong amount, however deep, never gets esig_B.
        v.zec_lock = ZecLock::WrongAmount;
        assert_eq!(chain(s, v), (s, vec![]));
        // ...and at t1 Bob cancels, so Alice's ZEC comes back through the refund.
        assert_eq!(
            chain(s, view(H_LOCK + 19).with_zec(ZecLock::WrongAmount)),
            (
                S::CancelBroadcast { h_lock: H_LOCK },
                vec![A::BroadcastCancel]
            )
        );
    }

    #[test]
    fn spec_5_3_r5_bob_refunds_within_one_block() {
        let mut v = view(121);
        v.cancel = TxStatus::Confirmed { height: 121 };
        // Seeing the cancel confirmed and broadcasting the refund happen in one step.
        assert_eq!(
            chain(S::CancelBroadcast { h_lock: H_LOCK }, v),
            (
                S::RefundBroadcast {
                    h_cancel: 121,
                    at_tip: 121
                },
                vec![A::BroadcastRefund]
            )
        );
        v.refund = TxStatus::Mempool;
        v.btc_tip = 122;
        assert_eq!(
            chain(
                S::RefundBroadcast {
                    h_cancel: 121,
                    at_tip: 121
                },
                v
            ),
            (
                S::RefundBroadcast {
                    h_cancel: 121,
                    at_tip: 122
                },
                vec![A::CpfpRefund]
            )
        );
        v.refund = TxStatus::Confirmed { height: 123 };
        v.btc_tip = 123;
        assert_eq!(
            chain(
                S::RefundBroadcast {
                    h_cancel: 121,
                    at_tip: 122
                },
                v
            ),
            (S::Done(Outcome::Refunded), vec![])
        );
    }

    /// S-03: Alice never pays. Back online long after, one step cancels.
    #[test]
    fn s_03_alice_never_pays_and_bob_catches_up_after_downtime() {
        assert_eq!(
            chain(S::EncSigSent { h_lock: H_LOCK }, view(400)).1,
            [A::BroadcastCancel]
        );
        let mut v = view(400);
        v.cancel = TxStatus::Confirmed { height: 125 };
        v.punish = TxStatus::Confirmed { height: 146 };
        assert_eq!(
            chain(S::EncSigSent { h_lock: H_LOCK }, v).0,
            S::Done(Outcome::Punished)
        );
    }

    #[test]
    fn spec_4_5_r1_redeem_in_mempool_is_enough_to_sweep() {
        let v = view(H_LOCK + 3).with_redeem(TxStatus::Mempool);
        assert_eq!(
            chain(S::CancelBroadcast { h_lock: H_LOCK }, v),
            (S::RedeemSeen, vec![A::SweepZec])
        );
    }

    #[test]
    fn aborts_only_before_the_lock() {
        assert_eq!(
            step_bob(S::Revealed, &I::PeerMisbehaved, &P).0,
            S::Aborted(AbortReason::PeerMisbehaved)
        );
        assert_eq!(
            step_bob(S::Committed, &I::SetupTimedOut, &P).0,
            S::Aborted(AbortReason::NoLock)
        );
        let s = S::BtcLockBroadcast;
        assert_eq!(
            step_bob(s, &I::PeerMisbehaved, &P),
            (s, vec![A::Alert(Alert::PeerMisbehavedAfterLock)])
        );
    }
}
