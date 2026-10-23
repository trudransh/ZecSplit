//! The runner against a fake two-source chain and an in-process maker running Bob's real session.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use async_trait::async_trait;
use bitcoin::hashes::Hash;
use bitcoin::secp256k1::Secp256k1;
use bitcoin::{
    Address, Amount, CompressedPublicKey, OutPoint, Psbt, ScriptBuf, Sequence, Transaction, TxIn,
    TxOut, Txid, WPubkeyHash, Witness, absolute, transaction,
};
use reddsa::orchard::SpendAuth;
use zecsplit_btc::sources::{BtcSource, SourceError, SourceTx};
use zecsplit_crypto::rand_core::OsRng;
use zecsplit_crypto::{Ask, Role};
use zecsplit_protocol::alice::{AliceInput, AliceState};
use zecsplit_protocol::machine::Outcome;
use zecsplit_protocol::messages::{Envelope, Message, Network, PROTOCOL_VERSION, Quote};
use zecsplit_protocol::recovery::SecretString;
use zecsplit_protocol::session::alice::AliceSession;
use zecsplit_protocol::session::bob::BobSession;
use zecsplit_protocol::session::{Core, Shared};
use zecsplit_store::Store;

use zecsplit_btc::sources::Broadcast;

use super::{AliceRunner, Maker, Ports, ZecSweep};

#[derive(Default)]
struct Chain {
    tip: u32,
    txs: HashMap<Txid, (Transaction, Option<u32>)>,
}

#[derive(Default)]
struct FakeChain(Mutex<Chain>);

impl FakeChain {
    fn put(&self, tx: Transaction) {
        self.0
            .lock()
            .unwrap()
            .txs
            .entry(tx.compute_txid())
            .or_insert((tx, None));
    }
    fn mine(&self) {
        let mut c = self.0.lock().unwrap();
        c.tip += 1;
        let tip = c.tip;
        for (_, h) in c.txs.values_mut() {
            h.get_or_insert(tip);
        }
    }
    fn get(&self, txid: &Txid) -> Option<Transaction> {
        self.0.lock().unwrap().txs.get(txid).map(|(t, _)| t.clone())
    }
}

#[async_trait]
impl BtcSource for FakeChain {
    async fn tip(&self) -> Result<u32, SourceError> {
        Ok(self.0.lock().unwrap().tip)
    }
    async fn tx(&self, txid: &Txid) -> Result<Option<SourceTx>, SourceError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .txs
            .get(txid)
            .map(|(tx, height)| SourceTx {
                tx: tx.clone(),
                height: *height,
            }))
    }
}

#[async_trait]
impl Broadcast for FakeChain {
    async fn broadcast(&self, tx: &Transaction) -> Result<(), SourceError> {
        self.put(tx.clone());
        Ok(())
    }
}

/// The maker: Bob's real session, answering over the INTERFACES §2 message flow.
struct FakeMaker<'a> {
    bob: Mutex<BobSession>,
    outbox: Mutex<VecDeque<Message>>,
    chain: &'a FakeChain,
}

impl FakeMaker<'_> {
    fn env(&self, message: Message) -> Envelope {
        let swap_id = self.bob.lock().unwrap().core.quote.swap_id;
        Envelope {
            v: PROTOCOL_VERSION,
            swap_id,
            message,
        }
    }
}

#[async_trait]
impl Maker for FakeMaker<'_> {
    async fn send(&self, msg: &Envelope) -> Result<Option<Envelope>, String> {
        let mut bob = self.bob.lock().unwrap();
        let reply = match &msg.message {
            Message::Commit(c) => {
                bob.core.on_commit(c).unwrap();
                Message::Commit(bob.core.commit_msg())
            }
            Message::Reveal(r) => {
                let reply = bob.core.reveal_msg().unwrap();
                bob.core.on_reveal(r).unwrap();
                let refund = addr(3);
                let (spk, v) = bob.lock_output(&refund).unwrap();
                let tpl = bob.lock_template(lock_psbt(spk, v), &refund).unwrap();
                self.outbox
                    .lock()
                    .unwrap()
                    .push_back(Message::LockTemplate(tpl));
                Message::Reveal(Box::new(reply))
            }
            Message::SetupSigsAlice(s) => {
                let reply = bob.on_setup_sigs_alice(s).unwrap();
                assert!(bob.may_broadcast_lock());
                self.chain
                    .put(bob.templates().unwrap().lock.unsigned_tx.clone());
                Message::SetupSigsBob(reply)
            }
            other => panic!("unexpected {other:?}"),
        };
        drop(bob);
        Ok(Some(self.env(reply)))
    }
    async fn next(&self, _: &[u8; 32]) -> Result<Option<Envelope>, String> {
        let next = self.outbox.lock().unwrap().pop_front();
        Ok(next.map(|m| self.env(m)))
    }
}

#[derive(Default)]
struct FakeZec(Mutex<Option<(bool, u32)>>);

#[async_trait]
impl ZecSweep for FakeZec {
    async fn sweep(&self, shared: &Shared, ask: &Ask) -> Result<String, String> {
        *self.0.lock().unwrap() = Some((signs_for(ask, shared), 0));
        Ok("zec-sweep".into())
    }
    async fn confirmations(&self, _: &Shared, _: &str) -> Result<Option<u32>, String> {
        Ok(self.0.lock().unwrap().as_ref().map(|(_, c)| *c))
    }
}

fn quote() -> Quote {
    Quote {
        protocol_version: 1,
        swap_id: [0x5a; 32],
        network: Network::Regtest,
        zec_amount: 100_000_000,
        btc_amount: 150_000,
        btc_fee_rate: 2,
        t1: 20,
        t2: 20,
        n_conf_btc: 1,
        n_conf_zec: 1,
        redeem_margin: 5,
        expires_at: u64::MAX,
    }
}

fn addr(seed: u8) -> String {
    let sk = bitcoin::secp256k1::SecretKey::from_slice(&[seed; 32]).unwrap();
    let pk = CompressedPublicKey(sk.public_key(&Secp256k1::new()));
    Address::p2wpkh(&pk, bitcoin::Network::Regtest).to_string()
}

fn lock_psbt(lock_spk: ScriptBuf, v_lock: Amount) -> Psbt {
    let tx = Transaction {
        version: transaction::Version::TWO,
        lock_time: absolute::LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::new(Txid::from_byte_array([1; 32]), 0),
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::new(),
        }],
        output: vec![TxOut {
            value: v_lock,
            script_pubkey: lock_spk,
        }],
    };
    let mut psbt = Psbt::from_unsigned_tx(tx).unwrap();
    psbt.inputs[0].witness_utxo = Some(TxOut {
        value: Amount::from_sat(1_000_000),
        script_pubkey: ScriptBuf::new_p2wpkh(&WPubkeyHash::from_byte_array([0xcc; 20])),
    });
    psbt
}

fn signs_for(ask: &Ask, shared: &Shared) -> bool {
    let sk = reddsa::SigningKey::<SpendAuth>::from_bytes(ask.expose_secret()).unwrap();
    <[u8; 32]>::from(reddsa::VerificationKey::from(&sk)) == shared.ak
}

fn pass() -> SecretString {
    SecretString::from("test passphrase".to_owned())
}

struct World<'a> {
    chain: &'a FakeChain,
    maker: FakeMaker<'a>,
    zec: FakeZec,
}

impl<'a> World<'a> {
    fn new(chain: &'a FakeChain) -> Self {
        let bob = Core::new(&mut OsRng, quote(), Role::Bob, addr(2)).unwrap();
        World {
            chain,
            maker: FakeMaker {
                bob: Mutex::new(BobSession::new(bob)),
                outbox: Mutex::new(VecDeque::new()),
                chain,
            },
            zec: FakeZec::default(),
        }
    }
}

/// Drives one swap; `resume_each_tick` rebuilds the runner from SQLite before every tick (S-07).
async fn run(
    world: &World<'_>,
    bob_sends_encsig: bool,
    resume_each_tick: bool,
) -> (AliceState, Shared) {
    let chain = world.chain;
    let sources: [&dyn BtcSource; 2] = [chain, chain];
    let ports = Ports {
        maker: &world.maker,
        sources: &sources,
        broadcast: chain,
        zec: &world.zec,
    };
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("engine.sqlite")).unwrap();
    let alice = Core::new(&mut OsRng, quote(), Role::Alice, addr(1)).unwrap();
    let mut runner = AliceRunner::start(
        &mut store,
        AliceSession::new(alice),
        pass(),
        "http://maker",
        0,
    )
    .unwrap();
    runner
        .feed(&mut store, &ports, AliceInput::Begin, 0)
        .await
        .unwrap();

    let mut encsig_sent = false;
    for tick in 1..120 {
        if resume_each_tick && runner.session.is_complete() {
            let row = store.active_swaps().unwrap().into_iter().next().unwrap();
            runner = AliceRunner::resume(&mut store, &row, pass(), tick)
                .unwrap()
                .unwrap();
            runner
                .feed(&mut store, &ports, AliceInput::Resume, tick)
                .await
                .unwrap();
        }
        runner.tick(&mut store, &ports, tick).await.unwrap();
        if runner.state.is_terminal() {
            break;
        }
        if runner.payment_request.is_some() && bob_sends_encsig && !encsig_sent {
            let esig = world.maker.bob.lock().unwrap().encsig().unwrap();
            world
                .maker
                .outbox
                .lock()
                .unwrap()
                .push_back(Message::RedeemEncSig(esig));
            encsig_sent = true;
        }
        let bob = world.maker.bob.lock().unwrap();
        if let Some(t) = bob.templates()
            && chain.get(&t.cancel.compute_txid()).is_some()
            && chain.get(&t.refund.compute_txid()).is_none()
        {
            chain.put(bob.signed_refund().unwrap());
        }
        drop(bob);
        if let Some((_, c)) = world.zec.0.lock().unwrap().as_mut() {
            *c += 1;
        }
        chain.mine();
    }
    (runner.state, runner.session.core.shared_key().unwrap())
}

#[tokio::test(flavor = "multi_thread")]
async fn s_01_runner_happy_path_redeems_and_bob_gets_ask() {
    let chain = FakeChain::default();
    let world = World::new(&chain);
    let (state, shared) = run(&world, true, false).await;
    assert_eq!(state, AliceState::Done(Outcome::Swapped));
    let bob = world.maker.bob.lock().unwrap();
    let redeem = world
        .chain
        .get(&bob.templates().unwrap().redeem.compute_txid())
        .expect("redeem on chain");
    assert!(signs_for(&bob.ask_from_redeem(&redeem).unwrap(), &shared));
}

#[tokio::test(flavor = "multi_thread")]
async fn s_04_runner_bob_silent_alice_cancels_and_sweeps_back() {
    let chain = FakeChain::default();
    let world = World::new(&chain);
    let (state, _) = run(&world, false, false).await;
    assert_eq!(state, AliceState::Done(Outcome::Refunded));
    assert_eq!(world.zec.0.lock().unwrap().map(|(ok, _)| ok), Some(true));
}

#[tokio::test(flavor = "multi_thread")]
async fn s_07_runner_resumes_from_sqlite_before_every_tick() {
    let chain = FakeChain::default();
    let world = World::new(&chain);
    let (state, _) = run(&world, true, true).await;
    assert_eq!(state, AliceState::Done(Outcome::Swapped));
}

#[test]
fn spec_6_3_r2_swap_stopped_before_setup_is_aborted_on_resume() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("engine.sqlite")).unwrap();
    let alice = Core::new(&mut OsRng, quote(), Role::Alice, addr(1)).unwrap();
    AliceRunner::start(
        &mut store,
        AliceSession::new(alice),
        pass(),
        "http://maker",
        0,
    )
    .unwrap();
    let row = store.active_swaps().unwrap().remove(0);
    assert!(
        AliceRunner::resume(&mut store, &row, pass(), 1)
            .unwrap()
            .is_none()
    );
    assert!(store.active_swaps().unwrap().is_empty());
}
