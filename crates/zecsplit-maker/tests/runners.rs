//! The engine's Alice runner against the maker's Bob runner, in-process, over a fake chain, wallet and Zcash node.

#![allow(clippy::unwrap_used, clippy::type_complexity)]

mod common;

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use bitcoin::secp256k1::Secp256k1;
use bitcoin::{Address, Amount, CompressedPublicKey, Psbt, ScriptBuf, Transaction, Txid};
use reddsa::orchard::SpendAuth;
use zecsplit_btc::sources::{Broadcast, BtcSource, SourceError, SourceTx};
use zecsplit_crypto::rand_core::OsRng;
use zecsplit_crypto::{Ask, Role};
use zecsplit_maker::runner::{self as bob, BobRunner, Wallet, ZecNode};
use zecsplit_protocol::alice::{AliceInput, AliceState};
use zecsplit_protocol::bob::{BobInput, BobState};
use zecsplit_protocol::machine::{Outcome, ZecLock};
use zecsplit_protocol::messages::{Network, Quote};
use zecsplit_protocol::session::alice::AliceSession;
use zecsplit_protocol::session::bob::BobSession;
use zecsplit_protocol::session::{Core, Shared};
use zecsplit_store::Store;
use zecsplitd::runner::{self as alice, AliceRunner, ZecSweep};

use common::{Link, pass};

#[derive(Default)]
struct FakeChain(Mutex<(u32, HashMap<Txid, (Transaction, Option<u32>)>)>);

impl FakeChain {
    fn mine(&self) {
        let mut c = self.0.lock().unwrap();
        c.0 += 1;
        let tip = c.0;
        for (_, h) in c.1.values_mut() {
            h.get_or_insert(tip);
        }
    }
}

#[async_trait]
impl BtcSource for FakeChain {
    async fn tip(&self) -> Result<u32, SourceError> {
        Ok(self.0.lock().unwrap().0)
    }
    async fn tx(&self, txid: &Txid) -> Result<Option<SourceTx>, SourceError> {
        let c = self.0.lock().unwrap();
        Ok(c.1.get(txid).map(|(tx, height)| SourceTx {
            tx: tx.clone(),
            height: *height,
        }))
    }
}

#[async_trait]
impl Broadcast for FakeChain {
    async fn broadcast(&self, tx: &Transaction) -> Result<(), SourceError> {
        let mut c = self.0.lock().unwrap();
        c.1.entry(tx.compute_txid()).or_insert((tx.clone(), None));
        Ok(())
    }
}

struct FakeWallet;

#[async_trait]
impl Wallet for FakeWallet {
    async fn refund_address(&self) -> Result<String, String> {
        Ok(addr(3))
    }
    async fn fund(&self, spk: &ScriptBuf, value: Amount) -> Result<Psbt, String> {
        Ok(lock_psbt(spk.clone(), value))
    }
    async fn sign(&self, psbt: &Psbt) -> Result<Transaction, String> {
        Ok(psbt.unsigned_tx.clone())
    }
}

/// One Zcash chain both parties see; `bob_blind` hides the lock from Bob (S-04).
#[derive(Default)]
struct FakeZcash {
    bob_blind: bool,
    paid: Mutex<Option<u32>>,
    sweeps: Mutex<Vec<(&'static str, bool, u32)>>,
}

impl FakeZcash {
    fn mine(&self) {
        if let Some(c) = self.paid.lock().unwrap().as_mut() {
            *c += 1;
        }
        for s in self.sweeps.lock().unwrap().iter_mut() {
            s.2 += 1;
        }
    }
    fn sweep_as(&self, who: &'static str, shared: &Shared, ask: &Ask) -> String {
        self.sweeps
            .lock()
            .unwrap()
            .push((who, signs_for(ask, shared), 0));
        format!("sweep-{who}")
    }
    fn confs(&self, txid: &str) -> Option<u32> {
        let sweeps = self.sweeps.lock().unwrap();
        sweeps
            .iter()
            .find(|s| format!("sweep-{}", s.0) == txid)
            .map(|s| s.2)
    }
}

#[async_trait]
impl ZecSweep for FakeZcash {
    async fn sweep(&self, shared: &Shared, ask: &Ask) -> Result<String, String> {
        Ok(self.sweep_as("alice", shared, ask))
    }
    async fn confirmations(&self, _: &Shared, txid: &str) -> Result<Option<u32>, String> {
        Ok(self.confs(txid))
    }
}

#[async_trait]
impl ZecNode for FakeZcash {
    async fn lock(&self, _: &Shared, _: u64, _: u32) -> Result<ZecLock, String> {
        Ok(match *self.paid.lock().unwrap() {
            Some(confirmations) if !self.bob_blind => ZecLock::Correct { confirmations },
            _ => ZecLock::Unseen,
        })
    }
    async fn sweep(&self, shared: &Shared, ask: &Ask) -> Result<String, String> {
        Ok(self.sweep_as("bob", shared, ask))
    }
    async fn confirmations(&self, _: &Shared, txid: &str) -> Result<Option<u32>, String> {
        Ok(self.confs(txid))
    }
}

fn quote() -> Quote {
    Quote {
        protocol_version: 1,
        swap_id: [0x77; 32],
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
    use bitcoin::hashes::Hash;
    use bitcoin::{OutPoint, Sequence, TxIn, TxOut, WPubkeyHash, Witness};
    let tx = Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: bitcoin::absolute::LockTime::ZERO,
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

/// Runs one swap to the end; `restart` rebuilds each runner from SQLite before every tick once it is saved (S-07).
async fn swap(zec: &FakeZcash, restart: bool) -> (AliceState, BobState) {
    let chain = FakeChain::default();
    let sources: [&dyn BtcSource; 2] = [&chain, &chain];
    let dirs = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut store_a = Store::open(&dirs.0.path().join("engine.sqlite")).unwrap();
    let mut store_b = Store::open(&dirs.1.path().join("maker.sqlite")).unwrap();

    let b = Core::new(&mut OsRng, quote(), Role::Bob, addr(2)).unwrap();
    let bob_runner =
        BobRunner::start(&mut store_b, BobSession::new(b), pass(), "http://maker", 0).unwrap();
    let link = Link {
        bob: tokio::sync::Mutex::new((bob_runner, store_b)),
        ports: bob::Ports {
            sources: &sources,
            broadcast: &chain,
            wallet: &FakeWallet,
            zec,
        },
    };
    let ports_a = alice::Ports {
        maker: &link,
        sources: &sources,
        broadcast: &chain,
        zec,
    };
    let a = Core::new(&mut OsRng, quote(), Role::Alice, addr(1)).unwrap();
    let mut alice = AliceRunner::start(
        &mut store_a,
        AliceSession::new(a),
        pass(),
        "http://maker",
        0,
    )
    .unwrap();
    alice
        .feed(&mut store_a, &ports_a, AliceInput::Begin, 0)
        .await
        .unwrap();

    for t in 1..200 {
        if restart {
            if let Some(row) = store_a.active_swaps().unwrap().pop()
                && store_a.secret(&row.id).unwrap().is_some()
                && let Some(r) = AliceRunner::resume(&mut store_a, &row, pass(), t).unwrap()
            {
                alice = r;
                alice
                    .feed(&mut store_a, &ports_a, AliceInput::Resume, t)
                    .await
                    .unwrap();
            }
            let mut guard = link.bob.lock().await;
            let (runner, store) = &mut *guard;
            if let Some(row) = store.active_swaps().unwrap().pop()
                && store.secret(&row.id).unwrap().is_some()
                && let Some(r) = BobRunner::resume(store, &row, pass(), t).unwrap()
            {
                *runner = r;
                runner
                    .feed(store, &link.ports, BobInput::Resume, t)
                    .await
                    .unwrap();
            }
        }
        alice.tick(&mut store_a, &ports_a, t).await.unwrap();
        if alice.payment_request.is_some() {
            zec.paid.lock().unwrap().get_or_insert(0);
        }
        {
            let mut guard = link.bob.lock().await;
            let (runner, store) = &mut *guard;
            runner.tick(store, &link.ports, t).await.unwrap();
        }
        let bob_state = link.bob.lock().await.0.state;
        if alice.state.is_terminal() && bob_state.is_terminal() {
            break;
        }
        chain.mine();
        zec.mine();
    }
    let bob_state = link.bob.lock().await.0.state;
    (alice.state, bob_state)
}

fn swept(zec: &FakeZcash) -> Vec<(&'static str, bool)> {
    let sweeps = zec.sweeps.lock().unwrap();
    sweeps.iter().map(|s| (s.0, s.1)).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn s_01_engine_and_maker_runners_swap() {
    let zec = FakeZcash::default();
    let states = swap(&zec, false).await;
    assert_eq!(
        states,
        (
            AliceState::Done(Outcome::Swapped),
            BobState::Done(Outcome::Swapped)
        )
    );
    assert_eq!(swept(&zec), [("bob", true)]);
}

#[tokio::test(flavor = "multi_thread")]
async fn s_04_maker_never_sees_zec_both_refund() {
    let zec = FakeZcash {
        bob_blind: true,
        ..FakeZcash::default()
    };
    let states = swap(&zec, false).await;
    assert_eq!(
        states,
        (
            AliceState::Done(Outcome::Refunded),
            BobState::Done(Outcome::Refunded)
        )
    );
    assert_eq!(swept(&zec), [("alice", true)]);
}

#[tokio::test(flavor = "multi_thread")]
async fn s_07_both_runners_restart_before_every_tick() {
    let zec = FakeZcash::default();
    let states = swap(&zec, true).await;
    assert_eq!(
        states,
        (
            AliceState::Done(Outcome::Swapped),
            BobState::Done(Outcome::Swapped)
        )
    );
    assert_eq!(swept(&zec), [("bob", true)]);
}
