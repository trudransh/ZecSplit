//! Bounded model check of both state machines: random walks over adversarial chain views and peer
//! inputs, every safety invariant checked after every step. Early timelocked broadcasts are not
//! checked: consensus rejects them.

use zecsplit_protocol::alice::{AliceAction, AliceInput, AliceState, step_alice};
use zecsplit_protocol::bob::{BobAction, BobInput, BobState, step_bob};
use zecsplit_protocol::machine::{ChainView, Outcome, Params, SweepStatus, TxStatus, ZecLock};
use zecsplit_protocol::timers::may_redeem;

const P: Params = Params {
    t1: 20,
    t2: 20,
    n_conf_btc: 2,
    n_conf_zec: 2,
    redeem_margin: 5,
};
const WALKS: u32 = 20_000;
const STEPS: u32 = 60;

/// xorshift64*: deterministic, so a failing seed replays.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn pick<T: Copy>(&mut self, xs: &[T]) -> T {
        xs[usize::try_from(self.below(xs.len() as u64)).unwrap_or(0)]
    }
}

/// A tip that only moves forward, mostly by a block, sometimes by a jump (downtime).
fn advance(r: &mut Rng, tip: u32) -> u32 {
    match r.below(10) {
        0..=5 => tip + 1,
        6 | 7 => tip,
        8 => tip + 5,
        _ => tip + 30,
    }
}

fn tx(r: &mut Rng, tip: u32) -> TxStatus {
    match r.below(6) {
        0 | 1 => TxStatus::Unseen,
        2 => TxStatus::Mempool,
        3 => TxStatus::SourcesDisagree,
        _ => TxStatus::Confirmed {
            height: tip.saturating_sub(u32::try_from(r.below(25)).unwrap_or(0)),
        },
    }
}

fn view(r: &mut Rng, tip: u32) -> ChainView {
    ChainView {
        btc_tip: tip,
        lock: tx(r, tip),
        redeem: tx(r, tip),
        cancel: tx(r, tip),
        refund: tx(r, tip),
        punish: tx(r, tip),
        zec_sweep: r.pick(&[
            SweepStatus::Pending,
            SweepStatus::Mempool,
            SweepStatus::Confirmed { confirmations: 0 },
            SweepStatus::Confirmed { confirmations: 5 },
            SweepStatus::NothingToSweep,
        ]),
        zec_lock: r.pick(&[
            ZecLock::Unseen,
            ZecLock::WrongAmount,
            ZecLock::Correct { confirmations: 0 },
            ZecLock::Correct { confirmations: 5 },
        ]),
    }
}

fn confirmed(s: TxStatus, tip: u32) -> Option<u32> {
    s.confirmed_height(tip, P.n_conf_btc)
}

/// What Alice has done so far that constrains what she may do next.
#[derive(Default)]
struct AliceHistory {
    shown: bool,
    redeemed: bool,
}

fn alice_input(r: &mut Rng, tip: u32) -> AliceInput {
    match r.below(12) {
        0 => AliceInput::Resume,
        1 => AliceInput::PeerCommitted,
        2 => AliceInput::PeerRevealVerified,
        3 => AliceInput::LockTemplateVerified,
        4 => AliceInput::PeerSigsVerified,
        5 => AliceInput::EncSigVerified,
        6 => AliceInput::PeerMisbehaved,
        7 => AliceInput::LockTimedOut,
        _ => AliceInput::Chain(view(r, tip)),
    }
}

fn h_lock_of(s: AliceState) -> Option<u32> {
    match s {
        AliceState::AwaitingZecPayment { h_lock }
        | AliceState::EncSigReceived { h_lock }
        | AliceState::RedeemBroadcast { h_lock, .. }
        | AliceState::CancelBroadcast { h_lock } => Some(h_lock),
        _ => None,
    }
}

/// Every Alice invariant for one step; `Err` names the broken rule.
fn check_alice(
    h: &mut AliceHistory,
    before: AliceState,
    input: &AliceInput,
    after: AliceState,
    acts: &[AliceAction],
) -> Result<(), String> {
    if before.is_terminal() && (after != before || !acts.is_empty()) {
        return Err("terminal state moved or acted".into());
    }
    let v = match input {
        AliceInput::Chain(v) => Some(v),
        _ => None,
    };
    for a in acts {
        match (a, v) {
            (AliceAction::ShowPaymentRequest, Some(v)) => {
                let Some(hl) = confirmed(v.lock, v.btc_tip) else {
                    return Err("§4.2-R2: payment request without a confirmed lock".into());
                };
                if !may_redeem(v.btc_tip, hl, P.t1, P.redeem_margin) {
                    return Err("§4.4-R1: payment request inside the margin".into());
                }
                h.shown = true;
            }
            (AliceAction::ShowPaymentRequest, None) => {
                if !matches!(before, AliceState::AwaitingZecPayment { .. }) {
                    return Err("payment request re-shown outside AwaitingZecPayment".into());
                }
                h.shown = true;
            }
            (AliceAction::BroadcastRedeem, Some(v)) => {
                let hl = h_lock_of(after).ok_or("redeem without h_lock")?;
                if matches!(before, AliceState::EncSigReceived { .. })
                    && !may_redeem(v.btc_tip, hl, P.t1, P.redeem_margin)
                {
                    return Err("§4.4-R1: first redeem broadcast inside the margin".into());
                }
                h.redeemed = true;
            }
            (AliceAction::BroadcastRedeem, None) => {
                if !h.redeemed && !matches!(before, AliceState::RedeemBroadcast { .. }) {
                    return Err("redeem broadcast out of nowhere".into());
                }
                h.redeemed = true;
            }
            (AliceAction::BroadcastCancel, Some(v)) => {
                if h.redeemed && !v.cancel.seen() {
                    return Err("ADR-033: cancel against her own pending redeem".into());
                }
            }
            (AliceAction::SweepZec, _) => {
                if !matches!(after, AliceState::RefundSeen) {
                    return Err("§5.3-R3: sweep without the refund (no s_b)".into());
                }
            }
            _ => {}
        }
    }
    if h.shown && matches!(after, AliceState::Aborted(_)) {
        return Err("funds safety: aborted after the payment request was shown".into());
    }
    if after == AliceState::Done(Outcome::Swapped) && before != after {
        let v = v.ok_or("Swapped without a chain view")?;
        if confirmed(v.redeem, v.btc_tip).is_none() {
            return Err("Swapped without a confirmed redeem".into());
        }
    }
    Ok(())
}

#[test]
fn alice_never_breaks_a_safety_invariant() {
    for seed in 1..=u64::from(WALKS) {
        let mut r = Rng(seed);
        let mut s = AliceState::Quoted;
        let mut h = AliceHistory::default();
        let mut tip = 100;
        let mut trace = Vec::new();
        for _ in 0..STEPS {
            tip = advance(&mut r, tip);
            let input = alice_input(&mut r, tip);
            let (next, acts) = step_alice(s, &input, &P);
            trace.push(format!("{s:?} --{input:?}--> {next:?} {acts:?}"));
            if let Err(e) = check_alice(&mut h, s, &input, next, &acts) {
                panic!("seed {seed}: {e}\n{}", trace.join("\n"));
            }
            s = next;
        }
    }
}

#[derive(Default)]
struct BobHistory {
    sigs: bool,
    locked: bool,
}

fn bob_input(r: &mut Rng, tip: u32) -> BobInput {
    match r.below(10) {
        0 => BobInput::Resume,
        1 => BobInput::PeerCommitted,
        2 => BobInput::PeerRevealVerified,
        3 => BobInput::PeerSigsVerified,
        4 => BobInput::PeerMisbehaved,
        5 => BobInput::SetupTimedOut,
        _ => BobInput::Chain(view(r, tip)),
    }
}

/// Every Bob invariant for one step.
fn check_bob(
    h: &mut BobHistory,
    before: BobState,
    input: &BobInput,
    after: BobState,
    acts: &[BobAction],
) -> Result<(), String> {
    if before.is_terminal() && (after != before || !acts.is_empty()) {
        return Err("terminal state moved or acted".into());
    }
    if *input == BobInput::PeerSigsVerified && before == BobState::Revealed {
        h.sigs = true;
    }
    let v = match input {
        BobInput::Chain(v) => Some(v),
        _ => None,
    };
    for a in acts {
        match (a, v) {
            (BobAction::BroadcastLock, _) => {
                if !h.sigs {
                    return Err("§3.5-R5: lock before Alice's signatures".into());
                }
                h.locked = true;
            }
            (BobAction::SendEncSig, Some(v)) => {
                let ok = matches!(v.zec_lock, ZecLock::Correct { confirmations }
                    if confirmations >= u32::from(P.n_conf_zec));
                if !ok {
                    return Err("§4.3-R2: esig_B without the exact, confirmed ZEC lock".into());
                }
            }
            (BobAction::SendEncSig, None) => {
                if !matches!(before, BobState::EncSigSent { .. }) {
                    return Err("esig_B re-sent outside EncSigSent".into());
                }
            }
            (BobAction::SweepZec, _) => {
                if !matches!(after, BobState::RedeemSeen) {
                    return Err("§4.5: sweep without seeing the redeem (no s_a)".into());
                }
            }
            _ => {}
        }
    }
    if h.locked && matches!(after, BobState::Aborted(_)) {
        return Err("funds safety: aborted after broadcasting tx_lock".into());
    }
    Ok(())
}

#[test]
fn bob_never_breaks_a_safety_invariant() {
    for seed in 1..=u64::from(WALKS) {
        let mut r = Rng(seed);
        let mut s = BobState::Quoted;
        let mut h = BobHistory::default();
        let mut tip = 100;
        let mut trace = Vec::new();
        for _ in 0..STEPS {
            tip = advance(&mut r, tip);
            let input = bob_input(&mut r, tip);
            let (next, acts) = step_bob(s, &input, &P);
            trace.push(format!("{s:?} --{input:?}--> {next:?} {acts:?}"));
            if let Err(e) = check_bob(&mut h, s, &input, next, &acts) {
                panic!("seed {seed}: {e}\n{}", trace.join("\n"));
            }
            s = next;
        }
    }
}
