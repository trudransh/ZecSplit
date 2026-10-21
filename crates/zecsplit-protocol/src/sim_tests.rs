//! In-process harness (T-031): Alice and Bob against each other on a simulated chain.
//! Actions of one machine become inputs of the other; each round mines one block. The
//! chain enforces what Bitcoin does: relative timelocks (BIP68) and that conflicting
//! spends of one output can't both confirm. Scenario IDs are TEST_PLAN §2.

use crate::alice::{AliceAction, AliceInput, AliceState, step_alice};
use crate::bob::{BobAction, BobInput, BobState, step_bob};
use crate::machine::{ChainView, Outcome, Params, SweepStatus, TxStatus, ZecLock};
use crate::timers::{cancel_valid, punish_valid};

const P: Params = Params {
    t1: 20,
    t2: 20,
    n_conf_btc: 1,
    n_conf_zec: 1,
    redeem_margin: 5,
};

#[derive(Default, Clone, Copy)]
struct Faults {
    alice_never_pays: bool,
    bob_silent_after_zec: bool,
    bob_never_refunds: bool,
    /// S-07: lose all actions of Alice's n-th / Bob's n-th step (a crash right after the
    /// write-ahead persist), then resume.
    crash_alice_at: Option<usize>,
    crash_bob_at: Option<usize>,
}

struct Sim {
    tip: u32,
    lock: TxStatus,
    redeem: TxStatus,
    cancel: TxStatus,
    refund: TxStatus,
    punish: TxStatus,
    zec_paid_at: Option<u32>,
    alice_sweep: SweepStatus,
    bob_sweep: SweepStatus,
    alice: AliceState,
    bob: BobState,
    faults: Faults,
    alice_steps: usize,
    bob_steps: usize,
}

impl Sim {
    fn new(faults: Faults) -> Self {
        Sim {
            tip: 100,
            lock: TxStatus::Unseen,
            redeem: TxStatus::Unseen,
            cancel: TxStatus::Unseen,
            refund: TxStatus::Unseen,
            punish: TxStatus::Unseen,
            zec_paid_at: None,
            alice_sweep: SweepStatus::Pending,
            bob_sweep: SweepStatus::Pending,
            alice: AliceState::Quoted,
            bob: BobState::Quoted,
            faults,
            alice_steps: 0,
            bob_steps: 0,
        }
    }

    fn h(s: TxStatus) -> Option<u32> {
        match s {
            TxStatus::Confirmed { height } => Some(height),
            TxStatus::Unseen | TxStatus::Mempool | TxStatus::SourcesDisagree => None,
        }
    }

    fn view(&self, sweep: SweepStatus, zec_lock: ZecLock) -> ChainView {
        ChainView {
            btc_tip: self.tip,
            lock: self.lock,
            redeem: self.redeem,
            cancel: self.cancel,
            refund: self.refund,
            punish: self.punish,
            zec_sweep: sweep,
            zec_lock,
        }
    }

    fn zec_lock(&self) -> ZecLock {
        match self.zec_paid_at {
            Some(at) => ZecLock::Correct {
                confirmations: self.tip.saturating_sub(at) + 1,
            },
            None => ZecLock::Unseen,
        }
    }

    fn alice(&mut self, input: AliceInput) {
        let (next, acts) = step_alice(self.alice, &input, &P);
        self.alice = next;
        self.alice_steps += 1;
        if self.faults.crash_alice_at == Some(self.alice_steps) {
            return self.alice(AliceInput::Resume);
        }
        for a in acts {
            match a {
                AliceAction::SendCommit => self.bob(BobInput::PeerCommitted),
                AliceAction::SendReveal => self.bob(BobInput::PeerRevealVerified),
                AliceAction::SendSetupSigs => self.bob(BobInput::PeerSigsVerified),
                AliceAction::ShowPaymentRequest if !self.faults.alice_never_pays => {
                    self.zec_paid_at.get_or_insert(self.tip + 1);
                }
                AliceAction::BroadcastRedeem
                    if self.redeem == TxStatus::Unseen && self.cancel == TxStatus::Unseen =>
                {
                    self.redeem = TxStatus::Mempool;
                }
                AliceAction::BroadcastCancel => self.try_cancel(),
                AliceAction::BroadcastPunish => {
                    if let Some(h_cancel) = Self::h(self.cancel)
                        && punish_valid(self.tip, h_cancel, P.t2)
                        && self.refund == TxStatus::Unseen
                    {
                        self.punish = TxStatus::Mempool;
                    }
                }
                AliceAction::SweepZec if self.alice_sweep == SweepStatus::Pending => {
                    self.alice_sweep = if self.zec_paid_at.is_some() {
                        SweepStatus::Mempool
                    } else {
                        SweepStatus::NothingToSweep
                    };
                }
                AliceAction::ShowPaymentRequest
                | AliceAction::BroadcastRedeem
                | AliceAction::SweepZec
                | AliceAction::HidePaymentRequest
                | AliceAction::CpfpRedeem
                | AliceAction::Alert(_) => {}
            }
        }
    }

    fn bob(&mut self, input: BobInput) {
        let (next, acts) = step_bob(self.bob, &input, &P);
        self.bob = next;
        self.bob_steps += 1;
        if self.faults.crash_bob_at == Some(self.bob_steps) {
            return self.bob(BobInput::Resume);
        }
        for a in acts {
            match a {
                BobAction::SendCommit => self.alice(AliceInput::PeerCommitted),
                BobAction::SendReveal => self.alice(AliceInput::PeerRevealVerified),
                BobAction::SendLockTemplate => self.alice(AliceInput::LockTemplateVerified),
                BobAction::SendSetupSigs => self.alice(AliceInput::PeerSigsVerified),
                BobAction::BroadcastLock if self.lock == TxStatus::Unseen => {
                    self.lock = TxStatus::Mempool
                }
                BobAction::SendEncSig if !self.faults.bob_silent_after_zec => {
                    self.alice(AliceInput::EncSigVerified)
                }
                BobAction::SweepZec if self.bob_sweep == SweepStatus::Pending => {
                    self.bob_sweep = SweepStatus::Mempool;
                }
                BobAction::BroadcastCancel => self.try_cancel(),
                BobAction::BroadcastRefund if !self.faults.bob_never_refunds => {
                    if Self::h(self.cancel).is_some()
                        && self.refund == TxStatus::Unseen
                        && self.punish == TxStatus::Unseen
                    {
                        self.refund = TxStatus::Mempool;
                    }
                }
                BobAction::BroadcastLock
                | BobAction::SendEncSig
                | BobAction::SweepZec
                | BobAction::BroadcastRefund
                | BobAction::CpfpRefund
                | BobAction::Alert(_) => {}
            }
        }
    }

    /// BIP68: valid only from `h_lock + t1`, and only while the redeem hasn't confirmed.
    fn try_cancel(&mut self) {
        if let Some(h_lock) = Self::h(self.lock)
            && cancel_valid(self.tip, h_lock, P.t1)
            && self.cancel == TxStatus::Unseen
            && Self::h(self.redeem).is_none()
        {
            self.cancel = TxStatus::Mempool;
            // The cancel and the redeem spend the same output; the cancel evicts it here.
            self.redeem = TxStatus::Unseen;
        }
    }

    fn mine(&mut self) {
        self.tip += 1;
        let tip = self.tip;
        for tx in [
            &mut self.lock,
            &mut self.redeem,
            &mut self.cancel,
            &mut self.refund,
            &mut self.punish,
        ] {
            if *tx == TxStatus::Mempool {
                *tx = TxStatus::Confirmed { height: tip };
            }
        }
        for sweep in [&mut self.alice_sweep, &mut self.bob_sweep] {
            match *sweep {
                SweepStatus::Mempool => *sweep = SweepStatus::Confirmed { confirmations: 1 },
                SweepStatus::Confirmed { confirmations } => {
                    *sweep = SweepStatus::Confirmed {
                        confirmations: confirmations + 1,
                    }
                }
                SweepStatus::Pending | SweepStatus::NothingToSweep => {}
            }
        }
    }

    fn run(faults: Faults) -> (AliceState, BobState) {
        let mut sim = Sim::new(faults);
        sim.alice(AliceInput::Begin);
        for _ in 0..200 {
            let alice_view = sim.view(sim.alice_sweep, ZecLock::Unseen);
            sim.alice(AliceInput::Chain(alice_view));
            let bob_view = sim.view(sim.bob_sweep, sim.zec_lock());
            sim.bob(BobInput::Chain(bob_view));
            if matches!(sim.alice, AliceState::Done(_)) && matches!(sim.bob, BobState::Done(_)) {
                break;
            }
            sim.mine();
        }
        (sim.alice, sim.bob)
    }
}

#[test]
fn s_01_happy_path_end_to_end() {
    assert_eq!(
        Sim::run(Faults::default()),
        (
            AliceState::Done(Outcome::Swapped),
            BobState::Done(Outcome::Swapped)
        )
    );
}

#[test]
fn s_03_alice_never_pays_everyone_refunded() {
    let faults = Faults {
        alice_never_pays: true,
        ..Faults::default()
    };
    assert_eq!(
        Sim::run(faults),
        (
            AliceState::Done(Outcome::Refunded),
            BobState::Done(Outcome::Refunded)
        )
    );
}

#[test]
fn s_04_bob_silent_after_zec_lock_alice_gets_zec_back() {
    let faults = Faults {
        bob_silent_after_zec: true,
        ..Faults::default()
    };
    assert_eq!(
        Sim::run(faults),
        (
            AliceState::Done(Outcome::Refunded),
            BobState::Done(Outcome::Refunded)
        )
    );
}

#[test]
fn s_05_bob_never_refunds_alice_punishes() {
    let faults = Faults {
        bob_silent_after_zec: true,
        bob_never_refunds: true,
        ..Faults::default()
    };
    assert_eq!(
        Sim::run(faults),
        (
            AliceState::Done(Outcome::Punished),
            BobState::Done(Outcome::Punished)
        )
    );
}

/// S-07: a crash after every single step, on either side, still ends in the same outcome.
#[test]
fn s_07_crash_after_every_step_still_swaps() {
    for n in 1..=60 {
        for faults in [
            Faults {
                crash_alice_at: Some(n),
                ..Faults::default()
            },
            Faults {
                crash_bob_at: Some(n),
                ..Faults::default()
            },
        ] {
            assert_eq!(
                Sim::run(faults),
                (
                    AliceState::Done(Outcome::Swapped),
                    BobState::Done(Outcome::Swapped)
                ),
                "crash at step {n}"
            );
        }
    }
}
