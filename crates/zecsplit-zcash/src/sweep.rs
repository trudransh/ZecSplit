//! Sweep of the lock notes with the joint spend key (PROTOCOL_SPEC §4.5-R3, T-012).

use std::convert::Infallible;
use std::path::Path;

use getrandom::SysRng;
use getrandom::rand_core::UnwrapErr;
use orchard::primitives::redpallas::{self, SpendAuth};
use pasta_curves::group::ff::PrimeField;
use pasta_curves::pallas;
use pczt::Pczt;
use pczt::roles::low_level_signer::{self, OrchardParseError};
use pczt::roles::prover::Prover;
use pczt::roles::signer::Signer;
use zcash_address::ZcashAddress;
use zcash_client_backend::data_api::wallet::input_selection::LockedInputPolicy;
use zcash_client_backend::data_api::wallet::{
    ConfirmationsPolicy, create_pczt_from_proposal, extract_and_store_transaction_from_pczt,
    propose_send_max_transfer,
};
use zcash_client_backend::data_api::{MaxSpendMode, WalletRead};
use zcash_client_backend::fees::StandardFeeRule;
use zcash_client_backend::proto::service::RawTransaction;
use zcash_client_backend::util::SystemClock;
use zcash_client_backend::wallet::OvkPolicy;
use zcash_client_sqlite::ReceivedNoteId;
use zcash_client_sqlite::wallet::commitment_tree;
use zcash_primitives::transaction::builder::{BundlePadding, cached_orchard_proving_key};
use zcash_primitives::transaction::components::orchard::bundle_version_for_branch;
use zcash_protocol::ShieldedPool;
use zcash_protocol::TxId;
use zcash_protocol::consensus::{BranchId, Parameters};
use zecsplit_crypto::Ask;
use zeroize::Zeroizing;

use crate::watch::{WatchError, synced_wallet};

#[derive(Debug, thiserror::Error)]
pub enum SweepError {
    #[error(transparent)]
    Watch(#[from] WatchError),
    #[error("recipient address: {0}")]
    BadRecipient(String),
    #[error("ask is not a Pallas scalar")]
    BadAsk,
    #[error("ask does not authorize the lock notes")]
    WrongKey,
    #[error("building the sweep: {0}")]
    Build(String),
    #[error("proving the sweep: {0}")]
    Prove(String),
    #[error("broadcast rejected: {0}")]
    Rejected(String),
}

impl From<OrchardParseError> for SweepError {
    fn from(e: OrchardParseError) -> Self {
        SweepError::Build(format!("{e:?}"))
    }
}

fn build_err(e: impl std::fmt::Debug) -> SweepError {
    SweepError::Build(format!("{e:?}"))
}

/// §4.5-R3: sweeps every lock note to `to` with `rsk_i = ask + α_i`, then broadcasts.
pub async fn sweep<P: Parameters + Clone + Send + 'static>(
    params: &P,
    db_path: &Path,
    ufvk: &str,
    birthday_height: u32,
    server: &str,
    ask: &Ask,
    to: &str,
) -> Result<TxId, SweepError> {
    let ask = Zeroizing::new(
        Option::<pallas::Scalar>::from(pallas::Scalar::from_repr(*ask.expose_secret()))
            .ok_or(SweepError::BadAsk)?,
    );
    let recipient =
        ZcashAddress::try_from_encoded(to).map_err(|e| SweepError::BadRecipient(e.to_string()))?;
    let (mut client, mut db) =
        synced_wallet(params, db_path, ufvk, birthday_height, server).await?;
    let account = *db
        .get_account_ids()
        .map_err(build_err)?
        .first()
        .ok_or_else(|| SweepError::Build("lock account missing".into()))?;

    let proposal = propose_send_max_transfer::<_, _, _, commitment_tree::Error>(
        &mut db,
        params,
        account,
        &[ShieldedPool::Ironwood, ShieldedPool::Orchard],
        &StandardFeeRule::Zip317,
        recipient,
        None,
        MaxSpendMode::Everything,
        ConfirmationsPolicy::MIN,
        &LockedInputPolicy::default(),
        None,
    )
    .map_err(build_err)?;
    let mut rng = UnwrapErr(SysRng);
    let pczt = create_pczt_from_proposal::<_, _, Infallible, _, Infallible, _>(
        &mut db,
        params,
        &SystemClock,
        &mut rng,
        account,
        OvkPolicy::Sender,
        &proposal,
        None,
        BundlePadding::DEFAULT,
    )
    .map_err(build_err)?;

    let pczt = prove(sign(pczt, &ask)?)?;
    let txid = extract_and_store_transaction_from_pczt::<_, ReceivedNoteId>(
        &mut db,
        &SystemClock,
        &mut rng,
        pczt,
        None,
        None,
    )
    .map_err(build_err)?;

    let tx = db
        .get_transaction(txid)
        .map_err(build_err)?
        .ok_or_else(|| SweepError::Build("sweep not stored".into()))?;
    let mut data = Vec::new();
    tx.write(&mut data).map_err(build_err)?;
    let sent = client
        .send_transaction(RawTransaction { data, height: 0 })
        .await
        .map_err(|e| WatchError::Server(e.to_string()))?
        .into_inner();
    if sent.error_code != 0 {
        return Err(SweepError::Rejected(sent.error_message));
    }
    Ok(txid)
}

/// §4.5-R3: signs every unsigned spend in both shielded bundles with `ask + α`.
fn sign(pczt: Pczt, ask: &pallas::Scalar) -> Result<Pczt, SweepError> {
    let sighash = Signer::new(pczt.clone())
        .map_err(build_err)?
        .shielded_sighash();
    let signed = low_level_signer::Signer::new(pczt)
        .sign_ironwood_with(|_, bundle, _| sign_bundle(bundle, ask, sighash))?
        .sign_orchard_with(|_, bundle, _| sign_bundle(bundle, ask, sighash))?;
    Ok(signed.finish())
}

/// §3.3-R1: a spend accepts the signature only if `ask` matches its `rk`.
fn sign_bundle(
    bundle: &mut orchard::pczt::Bundle,
    ask: &pallas::Scalar,
    sighash: [u8; 32],
) -> Result<(), SweepError> {
    for action in bundle.actions_mut() {
        if action.spend().spend_auth_sig().is_some() {
            continue;
        }
        let alpha = action.spend().alpha().ok_or(SweepError::WrongKey)?;
        let rsk = Zeroizing::new((*ask + alpha).to_repr());
        let rsk =
            redpallas::SigningKey::<SpendAuth>::try_from(*rsk).map_err(|_| SweepError::BadAsk)?;
        let sig = rsk.sign(UnwrapErr(SysRng), &sighash);
        action
            .apply_signature(sighash, sig)
            .map_err(|_| SweepError::WrongKey)?;
    }
    Ok(())
}

/// Creates the Ironwood and Orchard proofs with the circuit of the PCZT's branch.
fn prove(pczt: Pczt) -> Result<Pczt, SweepError> {
    let branch = BranchId::try_from(*pczt.global().consensus_branch_id())
        .map_err(|e| SweepError::Prove(e.to_string()))?;
    let key = |pool| {
        bundle_version_for_branch(branch, pool)
            .map(|v| cached_orchard_proving_key(v.circuit_version()))
            .ok_or_else(|| SweepError::Prove(format!("{pool:?} not active")))
    };
    let mut prover = Prover::new(pczt);
    if prover.requires_ironwood_proof() {
        prover = prover
            .create_ironwood_proof(UnwrapErr(SysRng), key(orchard::ValuePool::Ironwood)?)
            .map_err(|e| SweepError::Prove(format!("{e:?}")))?;
    }
    if prover.requires_orchard_proof() {
        prover = prover
            .create_orchard_proof(UnwrapErr(SysRng), key(orchard::ValuePool::Orchard)?)
            .map_err(|e| SweepError::Prove(format!("{e:?}")))?;
    }
    Ok(prover.finish())
}
