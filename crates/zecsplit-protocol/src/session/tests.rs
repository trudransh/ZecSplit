//! Both sessions against each other in-process with real keys, proofs and signatures.

use bitcoin::hashes::Hash;
use bitcoin::secp256k1::Secp256k1;
use bitcoin::{
    Address, Amount, CompressedPublicKey, OutPoint, Psbt, ScriptBuf, Sequence, Transaction, TxIn,
    TxOut, Txid, WPubkeyHash, Witness, absolute, transaction,
};
use reddsa::orchard::SpendAuth;
use zecsplit_btc::sign::{sig_in_witness, sighash, verify};
use zecsplit_crypto::rand_core::OsRng;
use zecsplit_crypto::{Ask, Role};

use super::alice::AliceSession;
use super::bob::BobSession;
use super::{Core, SessionError, Shared};
use crate::messages::{Network, Quote, SetupSigsAlice};

fn quote() -> Quote {
    Quote {
        protocol_version: 1,
        swap_id: [0x42; 32],
        network: Network::Regtest,
        zec_amount: 100_000_000,
        btc_amount: 150_000,
        btc_fee_rate: 2,
        t1: 20,
        t2: 20,
        n_conf_btc: 1,
        n_conf_zec: 1,
        redeem_margin: 5,
        expires_at: 1_800_000_000,
    }
}

fn addr(seed: u8) -> String {
    let sk = bitcoin::secp256k1::SecretKey::from_slice(&[seed; 32]).unwrap();
    let pk = CompressedPublicKey(sk.public_key(&Secp256k1::new()));
    Address::p2wpkh(&pk, bitcoin::Network::Regtest).to_string()
}

/// Bob's wallet: an unsigned `tx_lock` funding the lock output from one P2WPKH coin.
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

/// Runs §3.1–§3.5 and returns both sessions with the setup signatures exchanged.
fn setup() -> (AliceSession, BobSession, Shared) {
    let mut a = Core::new(&mut OsRng, quote(), Role::Alice, addr(1)).unwrap();
    let mut b = Core::new(&mut OsRng, quote(), Role::Bob, addr(2)).unwrap();
    a.on_commit(&b.commit_msg()).unwrap();
    b.on_commit(&a.commit_msg()).unwrap();
    let reveal_a = a.reveal_msg().unwrap();
    let reveal_b = b.reveal_msg().unwrap();
    let shared_a = a.on_reveal(&reveal_b).unwrap();
    let shared_b = b.on_reveal(&reveal_a).unwrap();
    assert_eq!(
        shared_a, shared_b,
        "both sides derive the same ak, σ, T, nk, rivk"
    );

    let mut alice = AliceSession::new(a);
    let mut bob = BobSession::new(b);
    let refund = addr(3);
    let (spk, v_lock) = bob.lock_output(&refund).unwrap();
    let tpl = bob.lock_template(lock_psbt(spk, v_lock), &refund).unwrap();
    let sigs_a = alice.on_lock_template(&tpl).unwrap();
    assert!(!bob.may_broadcast_lock());
    let sigs_b = bob.on_setup_sigs_alice(&sigs_a).unwrap();
    assert!(bob.may_broadcast_lock());
    alice.on_setup_sigs_bob(&sigs_b).unwrap();
    (alice, bob, shared_a)
}

fn ask_matches(ask: &Ask, shared: &Shared) -> bool {
    let sk = reddsa::SigningKey::<SpendAuth>::from_bytes(ask.expose_secret()).unwrap();
    <[u8; 32]>::from(reddsa::VerificationKey::from(&sk)) == shared.ak
}

fn spends_verify(
    tx: &Transaction,
    w: &ScriptBuf,
    value: Amount,
    keys: [&bitcoin::secp256k1::PublicKey; 2],
) -> bool {
    let digest = sighash(tx, w, value).unwrap();
    let wit = &tx.input[0].witness;
    keys.iter().all(|k| {
        let other = if keys[0] == *k { keys[1] } else { keys[0] };
        sig_in_witness(wit, k, other).is_some_and(|s| verify(k, &digest, &s))
    })
}

#[test]
fn spec_4_happy_path_redeem_reveals_s_a_and_bob_gets_ask() {
    let (mut alice, mut bob, shared) = setup();
    let esig = bob.encsig().unwrap();
    alice.on_encsig(&esig).unwrap();
    let redeem = alice.signed_redeem().unwrap();

    let txs = alice.templates().unwrap();
    let v_lock = txs.v_lock;
    let k_a = alice.core.own_btc_pk().unwrap();
    let k_b = bob.core.own_btc_pk().unwrap();
    assert!(spends_verify(&redeem, &txs.w_script, v_lock, [&k_a, &k_b]));

    let ask = bob.ask_from_redeem(&redeem).unwrap();
    assert!(
        ask_matches(&ask, &shared),
        "§4.5-R3: ask = σ(s_a + s_b) signs for ak"
    );
}

#[test]
fn spec_5_3_refund_reveals_s_b_and_alice_gets_ask() {
    let (alice, bob, shared) = setup();
    let cancel_a = alice.signed_cancel().unwrap();
    let cancel_b = bob.signed_cancel().unwrap();
    assert_eq!(cancel_a.input[0].witness, cancel_b.input[0].witness);

    let txs = bob.templates().unwrap();
    let k_a = alice.core.own_btc_pk().unwrap();
    let k_b = bob.core.own_btc_pk().unwrap();
    let v_lock = txs.v_lock;
    assert!(spends_verify(
        &cancel_a,
        &txs.w_script,
        v_lock,
        [&k_a, &k_b]
    ));

    let refund = bob.signed_refund().unwrap();
    assert!(spends_verify(
        &refund,
        &txs.w_script,
        txs.cancel.output[0].value,
        [&k_a, &k_b]
    ));
    let ask = alice.ask_from_refund(&refund).unwrap();
    assert!(
        ask_matches(&ask, &shared),
        "§5.3-R3: Alice sweeps back with σ(s_a + s_b)"
    );

    let punish = alice.signed_punish().unwrap();
    assert!(spends_verify(
        &punish,
        &txs.w_script,
        txs.cancel.output[0].value,
        [&k_a, &k_b]
    ));
}

#[test]
fn spec_3_5_r4_tampered_setup_sigs_are_rejected() {
    let mut a = Core::new(&mut OsRng, quote(), Role::Alice, addr(1)).unwrap();
    let mut b = Core::new(&mut OsRng, quote(), Role::Bob, addr(2)).unwrap();
    a.on_commit(&b.commit_msg()).unwrap();
    b.on_commit(&a.commit_msg()).unwrap();
    let (ra, rb) = (a.reveal_msg().unwrap(), b.reveal_msg().unwrap());
    a.on_reveal(&rb).unwrap();
    b.on_reveal(&ra).unwrap();
    let (mut alice, mut bob) = (AliceSession::new(a), BobSession::new(b));
    let (spk, v) = bob.lock_output(&addr(3)).unwrap();
    let tpl = bob.lock_template(lock_psbt(spk, v), &addr(3)).unwrap();
    let good = alice.on_lock_template(&tpl).unwrap();

    let mut bad_cancel = good.clone();
    bad_cancel.sig_cancel[10] ^= 1;
    assert!(matches!(
        bob.on_setup_sigs_alice(&bad_cancel),
        Err(SessionError::BadSignature(_))
    ));
    let mut bad_esig: SetupSigsAlice = good.clone();
    bad_esig.esig_refund[100] ^= 1;
    assert!(matches!(
        bob.on_setup_sigs_alice(&bad_esig),
        Err(SessionError::BadSignature(_))
    ));
    assert!(
        !bob.may_broadcast_lock(),
        "§3.5-R5: no lock without valid sigs"
    );
}

#[test]
fn spec_4_4_r2_tampered_encsig_is_rejected() {
    let (mut alice, mut bob, _) = setup();
    let mut esig = bob.encsig().unwrap();
    esig.esig_redeem[50] ^= 1;
    assert!(alice.on_encsig(&esig).is_err());
    assert!(
        alice.signed_redeem().is_err(),
        "no redeem without a verified esig_B"
    );
}

#[test]
fn spec_4_5_r1_unrelated_redeem_signature_does_not_yield_ask() {
    let (alice, bob, _) = setup();
    let cancel = alice.signed_cancel().unwrap();
    assert!(matches!(
        bob.ask_from_redeem(&cancel),
        Err(SessionError::OutOfOrder(_) | SessionError::Recover(_))
    ));
}

#[test]
fn spec_3_5_r1_lock_template_with_wrong_value_is_rejected() {
    let (mut alice, mut bob, _) = setup();
    let (spk, v) = bob.lock_output(&addr(3)).unwrap();
    let mut tpl = bob.lock_template(lock_psbt(spk, v), &addr(3)).unwrap();
    tpl.v_lock_sats += 1;
    assert!(matches!(
        alice.on_lock_template(&tpl),
        Err(SessionError::Template(_))
    ));
}

#[test]
fn spec_6_3_r2_restored_sessions_finish_the_swap_identically() {
    let (mut alice, mut bob, shared) = setup();
    let esig = bob.encsig().unwrap();
    alice.on_encsig(&esig).unwrap();

    let alice2 = AliceSession::restore(alice.save("http://maker").unwrap()).unwrap();
    let bob2 = BobSession::restore(bob.save("").unwrap()).unwrap();

    let redeem = alice.signed_redeem().unwrap();
    assert_eq!(
        alice2.signed_redeem().unwrap(),
        redeem,
        "deterministic replay"
    );
    assert!(ask_matches(
        &bob2.ask_from_redeem(&redeem).unwrap(),
        &shared
    ));
    assert_eq!(bob2.signed_refund().unwrap(), bob.signed_refund().unwrap());
    assert_eq!(
        alice2.signed_punish().unwrap(),
        alice.signed_punish().unwrap()
    );
    let refund = bob2.signed_refund().unwrap();
    assert!(ask_matches(
        &alice2.ask_from_refund(&refund).unwrap(),
        &shared
    ));
}

#[test]
fn spec_6_4_r1_alice_save_holds_every_required_blob() {
    let (alice, _, _) = setup();
    let bundle = alice.save("http://maker").unwrap();
    for name in crate::recovery::ALICE_REQUIRED {
        assert!(bundle.blob(name).is_some(), "{name}");
    }
    assert_eq!(bundle.maker_endpoint, "http://maker");
}

#[test]
fn restored_session_re_sends_the_same_reveal() {
    let (alice, _, _) = setup();
    let before = alice.save("").unwrap();
    let own = before.blob("own_reveal").unwrap().to_vec();
    let mut alice2 = AliceSession::restore(before).unwrap();
    let again = serde_json::to_vec(&alice2.core.reveal_msg().unwrap()).unwrap();
    assert_eq!(again, own);
}

#[test]
fn bundle_with_someone_elses_share_is_rejected() {
    let (alice, _, _) = setup();
    let mut bundle = alice.save("").unwrap();
    bundle.share = zecsplit_crypto::Share::from_bytes([1; 32]);
    assert!(matches!(
        AliceSession::restore(bundle),
        Err(SessionError::Key(_))
    ));
}

#[test]
fn saving_before_the_reveals_is_refused() {
    let core = Core::new(&mut OsRng, quote(), Role::Alice, addr(1)).unwrap();
    assert!(matches!(
        AliceSession::new(core).save(""),
        Err(SessionError::OutOfOrder(_))
    ));
}
