//! Alice, the taker (PROTOCOL_SPEC §6.1). She has ZEC and wants BTC.
//!
//! The rules this machine exists to enforce, in order of what costs funds:
//! - the payment request appears only once `tx_lock` is confirmed on both sources (§4.2-R2);
//! - she never redeems inside the margin (§4.4-R1) and never cancels against her own
//!   pending redeem;
//! - she cancels by `t1` (§5.3-R1) and punishes after `t2` if Bob hasn't refunded (§5.3-R4);
//! - `Aborted` is reachable only before the payment request (§6.1).

use crate::machine::{AbortReason, Alert, ChainView, Outcome, Params, SweepStatus, TxStatus};
use crate::timers::{cancel_valid, may_redeem, needs_cpfp, punish_valid};

/// §4.4-R3: CPFP a redeem still unconfirmed after this many blocks.
const REDEEM_CPFP_AFTER: u32 = 3;

/// §6.1. `BtcLockConfirmed` is folded into `AwaitingZecPayment`: a confirmed lock is
/// exactly what permits the payment request (ADR-033).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AliceState {
    /// Quote verified, keys generated, `C_A` sent (or to be re-sent).
    Quoted,
    /// `C_B` received, `Reveal_A` sent.
    Committed,
    /// `Reveal_B` verified. `sigs_sent` once `SetupSigsAlice` went out.
    Revealed {
        sigs_sent: bool,
    },
    /// Bob's cancel and punish signatures verified. Waiting for `tx_lock`.
    SigsExchanged,
    BtcLockSeen,
    AwaitingZecPayment {
        h_lock: u32,
    },
    EncSigReceived {
        h_lock: u32,
    },
    RedeemBroadcast {
        h_lock: u32,
        at_tip: u32,
    },
    CancelBroadcast {
        h_lock: u32,
    },
    Cancelled {
        h_cancel: u32,
    },
    /// Bob's refund confirmed: `s_b` is recoverable (§5.3-R3). Sweeping the ZEC back.
    RefundSeen,
    ZecSweepBroadcast,
    PunishBroadcast {
        h_cancel: u32,
    },
    Done(Outcome),
    Aborted(AbortReason),
}

/// Everything the shell can tell Alice. Messages arrive already verified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AliceInput {
    Begin,
    /// Engine restart: re-emit this state's (idempotent) duties.
    Resume,
    PeerCommitted,
    PeerRevealVerified,
    LockTemplateVerified,
    PeerSigsVerified,
    EncSigVerified,
    /// A message failed verification.
    PeerMisbehaved,
    /// The shell's wall-clock timer: Bob never locked BTC.
    LockTimedOut,
    Chain(ChainView),
}

/// What the shell must do, after persisting the new state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AliceAction {
    SendCommit,
    SendReveal,
    SendSetupSigs,
    ShowPaymentRequest,
    HidePaymentRequest,
    BroadcastRedeem,
    CpfpRedeem,
    BroadcastCancel,
    /// Recover `s_b` from the refund and sweep the lock notes back (§5.3-R3).
    SweepZec,
    BroadcastPunish,
    Alert(Alert),
}

pub fn step_alice(
    state: AliceState,
    input: &AliceInput,
    p: &Params,
) -> (AliceState, Vec<AliceAction>) {
    use AliceAction as A;
    use AliceInput as I;
    use AliceState as S;

    if let S::Done(_) | S::Aborted(_) = state {
        return (state, vec![]);
    }
    match *input {
        I::Chain(view) => on_chain(state, &view, p),
        I::Resume => (state, duties(state)),
        I::Begin => match state {
            S::Quoted => (state, vec![A::SendCommit]),
            _ => (state, vec![]),
        },
        I::PeerMisbehaved => {
            if before_payment_request(state) {
                (S::Aborted(AbortReason::PeerMisbehaved), vec![])
            } else {
                // Funds may be locked: only the timeouts can end this swap now.
                (state, vec![A::Alert(Alert::PeerMisbehavedAfterLock)])
            }
        }
        I::LockTimedOut => match state {
            S::Quoted | S::Committed | S::Revealed { .. } | S::SigsExchanged | S::BtcLockSeen => {
                (S::Aborted(AbortReason::NoLock), vec![])
            }
            _ => (state, vec![]),
        },
        I::PeerCommitted => match state {
            S::Quoted => (S::Committed, vec![A::SendReveal]),
            _ => (state, vec![]),
        },
        I::PeerRevealVerified => match state {
            S::Committed => (S::Revealed { sigs_sent: false }, vec![]),
            _ => (state, vec![]),
        },
        I::LockTemplateVerified => match state {
            S::Revealed { sigs_sent: false } => {
                (S::Revealed { sigs_sent: true }, vec![A::SendSetupSigs])
            }
            _ => (state, vec![]),
        },
        I::PeerSigsVerified => match state {
            S::Revealed { sigs_sent: true } => (S::SigsExchanged, vec![]),
            _ => (state, vec![]),
        },
        I::EncSigVerified => match state {
            S::AwaitingZecPayment { h_lock } => (S::EncSigReceived { h_lock }, vec![]),
            _ => (state, vec![]),
        },
    }
}

fn before_payment_request(state: AliceState) -> bool {
    use AliceState as S;
    match state {
        S::Quoted | S::Committed | S::Revealed { .. } | S::SigsExchanged | S::BtcLockSeen => true,
        S::AwaitingZecPayment { .. }
        | S::EncSigReceived { .. }
        | S::RedeemBroadcast { .. }
        | S::CancelBroadcast { .. }
        | S::Cancelled { .. }
        | S::RefundSeen
        | S::ZecSweepBroadcast
        | S::PunishBroadcast { .. }
        | S::Done(_)
        | S::Aborted(_) => false,
    }
}

/// Idempotent duties to repeat after a restart.
fn duties(state: AliceState) -> Vec<AliceAction> {
    use AliceAction as A;
    use AliceState as S;
    match state {
        S::Quoted => vec![A::SendCommit],
        S::Committed => vec![A::SendReveal],
        S::Revealed { sigs_sent: true } => vec![A::SendSetupSigs],
        S::AwaitingZecPayment { .. } => vec![A::ShowPaymentRequest],
        S::RedeemBroadcast { .. } => vec![A::BroadcastRedeem],
        S::CancelBroadcast { .. } => vec![A::BroadcastCancel],
        S::RefundSeen => vec![A::SweepZec],
        S::PunishBroadcast { .. } => vec![A::BroadcastPunish],
        S::Revealed { sigs_sent: false }
        | S::SigsExchanged
        | S::BtcLockSeen
        | S::EncSigReceived { .. }
        | S::Cancelled { .. }
        | S::ZecSweepBroadcast
        | S::Done(_)
        | S::Aborted(_) => vec![],
    }
}

fn on_chain(state: AliceState, v: &ChainView, p: &Params) -> (AliceState, Vec<AliceAction>) {
    use AliceAction as A;
    use AliceState as S;
    let tip = v.btc_tip;

    match state {
        S::Quoted | S::Committed | S::Revealed { .. } | S::Done(_) | S::Aborted(_) => {
            (state, vec![])
        }

        S::SigsExchanged | S::BtcLockSeen => match v.lock {
            TxStatus::SourcesDisagree => (state, vec![A::Alert(Alert::BtcSourcesDisagree)]),
            TxStatus::Unseen => (S::SigsExchanged, vec![]),
            TxStatus::Mempool | TxStatus::Confirmed { .. } => {
                match v.lock.confirmed_height(tip, p.n_conf_btc) {
                    Some(h_lock) => {
                        let next = S::AwaitingZecPayment { h_lock };
                        let (s, mut acts) = on_chain(next, v, p);
                        // §4.2-R2: the request appears only now, and only if it can still
                        // finish safely; otherwise the deadline logic above already took over.
                        if s == next {
                            acts.insert(0, A::ShowPaymentRequest);
                        }
                        (s, acts)
                    }
                    None => (S::BtcLockSeen, vec![]),
                }
            }
        },

        S::AwaitingZecPayment { h_lock } => {
            if let Some(done) = lock_phase_outcome(h_lock, v, p) {
                return done;
            }
            // ADV-09: the lock was reorged out. Hide the request and wait again.
            if v.lock.confirmed_height(tip, p.n_conf_btc).is_none() {
                return (S::BtcLockSeen, vec![A::HidePaymentRequest]);
            }
            if cancel_valid(tip, h_lock, p.t1) {
                return (
                    S::CancelBroadcast { h_lock },
                    vec![A::HidePaymentRequest, A::BroadcastCancel],
                );
            }
            if !may_redeem(tip, h_lock, p.t1, p.redeem_margin) {
                // Paying now could never be redeemed safely (§5.4).
                return (
                    state,
                    vec![A::HidePaymentRequest, A::Alert(Alert::MarginTooSmall)],
                );
            }
            (state, vec![])
        }

        S::EncSigReceived { h_lock } => {
            if let Some(done) = lock_phase_outcome(h_lock, v, p) {
                return done;
            }
            if may_redeem(tip, h_lock, p.t1, p.redeem_margin) {
                (
                    S::RedeemBroadcast {
                        h_lock,
                        at_tip: tip,
                    },
                    vec![A::BroadcastRedeem],
                )
            } else if cancel_valid(tip, h_lock, p.t1) {
                // §5.4: never reveal s_a now; take the refund path.
                (S::CancelBroadcast { h_lock }, vec![A::BroadcastCancel])
            } else {
                (state, vec![A::Alert(Alert::MarginTooSmall)])
            }
        }

        S::RedeemBroadcast { h_lock, at_tip } => {
            if let Some(done) = lock_phase_outcome(h_lock, v, p) {
                return done;
            }
            // Never cancel against our own pending redeem: fee-bump it instead.
            match v.redeem {
                TxStatus::Unseen | TxStatus::SourcesDisagree => (state, vec![A::BroadcastRedeem]),
                TxStatus::Mempool if needs_cpfp(tip, at_tip, REDEEM_CPFP_AFTER) => (
                    S::RedeemBroadcast {
                        h_lock,
                        at_tip: tip,
                    },
                    vec![A::CpfpRedeem],
                ),
                TxStatus::Mempool | TxStatus::Confirmed { .. } => (state, vec![]),
            }
        }

        S::CancelBroadcast { .. } => {
            if v.redeem.confirmed_height(tip, p.n_conf_btc).is_some() {
                return (S::Done(Outcome::Swapped), vec![]);
            }
            match v.cancel {
                TxStatus::Confirmed { height } => on_chain(S::Cancelled { h_cancel: height }, v, p),
                TxStatus::Unseen | TxStatus::SourcesDisagree => (state, vec![A::BroadcastCancel]),
                TxStatus::Mempool => (state, vec![]),
            }
        }

        S::Cancelled { h_cancel } | S::PunishBroadcast { h_cancel } => {
            if v.punish.confirmed_height(tip, p.n_conf_btc).is_some() {
                return (S::Done(Outcome::Punished), vec![]);
            }
            if let TxStatus::Confirmed { .. } = v.refund {
                return on_chain(S::RefundSeen, v, p);
            }
            if v.punish.seen() {
                return (S::PunishBroadcast { h_cancel }, vec![]);
            }
            if punish_valid(tip, h_cancel, p.t2) {
                return (S::PunishBroadcast { h_cancel }, vec![A::BroadcastPunish]);
            }
            (state, vec![])
        }

        S::RefundSeen | S::ZecSweepBroadcast => match v.zec_sweep {
            SweepStatus::NothingToSweep => (S::Done(Outcome::Refunded), vec![]),
            SweepStatus::Confirmed { confirmations }
                if confirmations >= u32::from(p.n_conf_zec) =>
            {
                (S::Done(Outcome::Refunded), vec![])
            }
            SweepStatus::Confirmed { .. } | SweepStatus::Mempool => (S::ZecSweepBroadcast, vec![]),
            SweepStatus::Pending => (S::RefundSeen, vec![A::SweepZec]),
        },
    }
}

/// Facts that end the lock phase whatever Alice was doing: her redeem confirmed, or a
/// cancel (hers or Bob's) is on chain.
fn lock_phase_outcome(
    h_lock: u32,
    v: &ChainView,
    p: &Params,
) -> Option<(AliceState, Vec<AliceAction>)> {
    if v.redeem.confirmed_height(v.btc_tip, p.n_conf_btc).is_some() {
        return Some((AliceState::Done(Outcome::Swapped), vec![]));
    }
    if let TxStatus::Confirmed { height } = v.cancel {
        return Some(on_chain(AliceState::Cancelled { h_cancel: height }, v, p));
    }
    if v.cancel.seen() {
        return Some((AliceState::CancelBroadcast { h_lock }, vec![]));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use AliceAction as A;
    use AliceInput as I;
    use AliceState as S;

    // regtest (§8): t1 = t2 = 20, margin 5. n_conf_btc = 2 so "seen" and "confirmed" differ.
    const P: Params = Params {
        t1: 20,
        t2: 20,
        n_conf_btc: 2,
        n_conf_zec: 1,
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
        }
    }

    fn chain(state: AliceState, v: ChainView) -> (AliceState, Vec<AliceAction>) {
        step_alice(state, &I::Chain(v), &P)
    }

    #[test]
    fn s_01_happy_path() {
        let mut s = S::Quoted;
        let mut log = vec![];
        for input in [
            I::Begin,
            I::PeerCommitted,
            I::PeerRevealVerified,
            I::LockTemplateVerified,
            I::PeerSigsVerified,
        ] {
            let (next, acts) = step_alice(s, &input, &P);
            s = next;
            log.extend(acts);
        }
        assert_eq!(s, S::SigsExchanged);
        assert_eq!(log, [A::SendCommit, A::SendReveal, A::SendSetupSigs]);

        let mut mempool = view(H_LOCK);
        mempool.lock = TxStatus::Mempool;
        assert_eq!(chain(s, mempool), (S::BtcLockSeen, vec![]));
        assert_eq!(
            chain(S::BtcLockSeen, view(H_LOCK + 1)),
            (
                S::AwaitingZecPayment { h_lock: H_LOCK },
                vec![A::ShowPaymentRequest]
            )
        );
        let (s, _) = step_alice(
            S::AwaitingZecPayment { h_lock: H_LOCK },
            &I::EncSigVerified,
            &P,
        );
        assert_eq!(s, S::EncSigReceived { h_lock: H_LOCK });
        let (s, acts) = chain(s, view(H_LOCK + 3));
        assert_eq!(acts, [A::BroadcastRedeem]);
        let mut redeemed = view(H_LOCK + 5);
        redeemed.redeem = TxStatus::Confirmed { height: H_LOCK + 4 };
        assert_eq!(chain(s, redeemed), (S::Done(Outcome::Swapped), vec![]));
    }

    #[test]
    fn spec_4_2_r2_no_payment_request_before_two_sources_confirm() {
        // One confirmation of the two required.
        assert_eq!(
            chain(S::BtcLockSeen, view(H_LOCK)),
            (S::BtcLockSeen, vec![])
        );
        assert_eq!(
            chain(
                S::BtcLockSeen,
                view(H_LOCK + 1).with_lock(TxStatus::SourcesDisagree)
            ),
            (S::BtcLockSeen, vec![A::Alert(Alert::BtcSourcesDisagree)])
        );
    }

    #[test]
    fn adv_09_reorged_lock_hides_the_request() {
        let v = view(H_LOCK + 2).with_lock(TxStatus::Mempool);
        assert_eq!(
            chain(S::AwaitingZecPayment { h_lock: H_LOCK }, v),
            (S::BtcLockSeen, vec![A::HidePaymentRequest])
        );
    }

    #[test]
    fn spec_4_4_r1_refuses_redeem_inside_margin_adv_06() {
        let s = S::EncSigReceived { h_lock: H_LOCK };
        // h_lock + t1 = 120: redeem allowed up to tip 114.
        assert_eq!(chain(s, view(114)).1, [A::BroadcastRedeem]);
        assert_eq!(
            chain(s, view(115)),
            (s, vec![A::Alert(Alert::MarginTooSmall)])
        );
        // §5.4: inside the margin she waits for t1, then cancels; s_a is never revealed.
        assert_eq!(
            chain(s, view(119)),
            (
                S::CancelBroadcast { h_lock: H_LOCK },
                vec![A::BroadcastCancel]
            )
        );
    }

    #[test]
    fn spec_5_3_r1_alice_cancels_by_t1_plus_1() {
        let s = S::AwaitingZecPayment { h_lock: H_LOCK };
        assert_eq!(chain(s, view(118)).0, s);
        assert_eq!(
            chain(s, view(119)),
            (
                S::CancelBroadcast { h_lock: H_LOCK },
                vec![A::HidePaymentRequest, A::BroadcastCancel]
            )
        );
    }

    #[test]
    fn spec_5_3_r4_punish_after_t2() {
        let mut v = view(138);
        v.cancel = TxStatus::Confirmed { height: 120 };
        let s = S::Cancelled { h_cancel: 120 };
        assert_eq!(chain(s, v), (s, vec![]));
        v.btc_tip = 139;
        assert_eq!(
            chain(s, v),
            (
                S::PunishBroadcast { h_cancel: 120 },
                vec![A::BroadcastPunish]
            )
        );
        v.punish = TxStatus::Confirmed { height: 140 };
        v.btc_tip = 141;
        assert_eq!(
            chain(S::PunishBroadcast { h_cancel: 120 }, v),
            (S::Done(Outcome::Punished), vec![])
        );
    }

    #[test]
    fn spec_4_4_r3_cpfp_after_three_blocks_and_never_self_cancel() {
        let s = S::RedeemBroadcast {
            h_lock: H_LOCK,
            at_tip: 110,
        };
        let v = view(112).with_redeem(TxStatus::Mempool);
        assert_eq!(chain(s, v), (s, vec![]));
        let (s2, acts) = chain(s, view(113).with_redeem(TxStatus::Mempool));
        assert_eq!(acts, [A::CpfpRedeem]);
        // Past t1 with the redeem pending: still no cancel from Alice.
        let (_, acts) = chain(s2, view(125).with_redeem(TxStatus::Mempool));
        assert!(!acts.contains(&A::BroadcastCancel));
    }

    /// §6.3-R2 / S-07: back online after Bob cancelled and refunded. One step takes Alice
    /// all the way to sweeping her ZEC back.
    #[test]
    fn spec_6_3_r2_offline_catch_up_jumps_to_the_sweep() {
        let mut v = view(150);
        v.cancel = TxStatus::Confirmed { height: 120 };
        v.refund = TxStatus::Confirmed { height: 121 };
        assert_eq!(
            chain(S::AwaitingZecPayment { h_lock: H_LOCK }, v),
            (S::RefundSeen, vec![A::SweepZec])
        );
        v.zec_sweep = SweepStatus::Confirmed { confirmations: 1 };
        assert_eq!(
            chain(S::ZecSweepBroadcast, v),
            (S::Done(Outcome::Refunded), vec![])
        );
    }

    #[test]
    fn spec_6_1_aborted_only_before_the_payment_request() {
        assert_eq!(
            step_alice(S::BtcLockSeen, &I::PeerMisbehaved, &P).0,
            S::Aborted(AbortReason::PeerMisbehaved)
        );
        let s = S::AwaitingZecPayment { h_lock: H_LOCK };
        assert_eq!(
            step_alice(s, &I::PeerMisbehaved, &P),
            (s, vec![A::Alert(Alert::PeerMisbehavedAfterLock)])
        );
        assert_eq!(step_alice(s, &I::LockTimedOut, &P).0, s);
    }

    #[test]
    fn s_02_maker_never_locks() {
        assert_eq!(
            step_alice(S::SigsExchanged, &I::LockTimedOut, &P),
            (S::Aborted(AbortReason::NoLock), vec![])
        );
    }

    #[test]
    fn spec_6_3_r1_resume_repeats_only_idempotent_duties() {
        assert_eq!(step_alice(S::Quoted, &I::Resume, &P).1, [A::SendCommit]);
        assert_eq!(
            step_alice(
                S::RedeemBroadcast {
                    h_lock: H_LOCK,
                    at_tip: 110
                },
                &I::Resume,
                &P
            )
            .1,
            [A::BroadcastRedeem]
        );
        assert!(
            step_alice(S::Done(Outcome::Swapped), &I::Resume, &P)
                .1
                .is_empty()
        );
    }

    impl ChainView {
        fn with_lock(mut self, s: TxStatus) -> Self {
            self.lock = s;
            self
        }
        fn with_redeem(mut self, s: TxStatus) -> Self {
            self.redeem = s;
            self
        }
    }
}
