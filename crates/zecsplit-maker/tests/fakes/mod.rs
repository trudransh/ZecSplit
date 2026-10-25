//! In-process stand-ins for the chains, the maker's wallet and Zcash, shared by the runner tests.

#![allow(dead_code, clippy::type_complexity)]

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use bitcoin::secp256k1::Secp256k1;
use bitcoin::{Address, Amount, CompressedPublicKey, Psbt, ScriptBuf, Transaction, Txid};
use reddsa::orchard::SpendAuth;
use zecsplit_btc::sources::{Broadcast, BtcSource, SourceError, SourceTx};
use zecsplit_crypto::Ask;
use zecsplit_maker::runner::{Wallet, ZecNode};
use zecsplit_protocol::machine::ZecLock;
use zecsplit_protocol::messages::{Network, Quote};
use zecsplit_protocol::session::Shared;
use zecsplitd::runner::ZecSweep;

#[derive(Default)]
pub struct FakeChain(pub Mutex<(u32, HashMap<Txid, (Transaction, Option<u32>)>)>);

impl FakeChain {
    pub fn mine(&self) {
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

/// Bob's line to `FakeChain` that, when `blocked`, carries only his `tx_lock` (ADV-07).
pub struct Censor<'a> {
    pub chain: &'a FakeChain,
    pub blocked: bool,
    pub lock: Mutex<Option<Txid>>,
}

#[async_trait]
impl Broadcast for Censor<'_> {
    async fn broadcast(&self, tx: &Transaction) -> Result<(), SourceError> {
        let txid = tx.compute_txid();
        if self.blocked && *self.lock.lock().unwrap().get_or_insert(txid) != txid {
            return Ok(());
        }
        self.chain.broadcast(tx).await
    }
}

pub struct FakeWallet;

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

/// One Zcash chain both parties see; `bob_blind` hides the lock from Bob (S-04), `wrong_amount` mispays it (ADV-05), `bob_delay` shows it to Bob only at that many confirmations (ADV-06).
#[derive(Default)]
pub struct FakeZcash {
    pub bob_blind: bool,
    pub wrong_amount: bool,
    pub bob_delay: u32,
    pub paid: Mutex<Option<u32>>,
    pub sweeps: Mutex<Vec<(&'static str, bool, u32)>>,
}

impl FakeZcash {
    pub fn mine(&self) {
        if let Some(c) = self.paid.lock().unwrap().as_mut() {
            *c += 1;
        }
        for s in self.sweeps.lock().unwrap().iter_mut() {
            s.2 += 1;
        }
    }
    pub fn sweep_as(&self, who: &'static str, shared: &Shared, ask: &Ask) -> String {
        self.sweeps
            .lock()
            .unwrap()
            .push((who, signs_for(ask, shared), 0));
        format!("sweep-{who}")
    }
    pub fn confs(&self, txid: &str) -> Option<u32> {
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
    async fn held(&self, _: &Shared) -> Result<u64, String> {
        Ok(self.paid.lock().unwrap().map_or(0, |_| quote().zec_amount))
    }
}

#[async_trait]
impl ZecNode for FakeZcash {
    async fn lock(&self, _: &Shared, _: u64, _: u32) -> Result<ZecLock, String> {
        Ok(match *self.paid.lock().unwrap() {
            Some(_) if self.wrong_amount => ZecLock::WrongAmount,
            Some(confirmations) if !self.bob_blind && confirmations >= self.bob_delay => {
                ZecLock::Correct { confirmations }
            }
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

pub fn quote() -> Quote {
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

pub fn addr(seed: u8) -> String {
    let sk = bitcoin::secp256k1::SecretKey::from_slice(&[seed; 32]).unwrap();
    let pk = CompressedPublicKey(sk.public_key(&Secp256k1::new()));
    Address::p2wpkh(&pk, bitcoin::Network::Regtest).to_string()
}

pub fn lock_psbt(lock_spk: ScriptBuf, v_lock: Amount) -> Psbt {
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

pub fn signs_for(ask: &Ask, shared: &Shared) -> bool {
    let sk = reddsa::SigningKey::<SpendAuth>::from_bytes(ask.expose_secret()).unwrap();
    <[u8; 32]>::from(reddsa::VerificationKey::from(&sk)) == shared.ak
}
