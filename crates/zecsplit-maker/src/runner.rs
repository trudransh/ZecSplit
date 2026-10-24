//! Bob's swap runner: answers the taker and steps his state machine on chain snapshots, persisting before acting (§6.3, ADR-033).

use std::collections::VecDeque;

use async_trait::async_trait;
use bitcoin::{Amount, Psbt, ScriptBuf, Transaction};
use zecsplit_btc::sources::{Broadcast, BtcSource, fetch_any, swap_status, tip_on_all};
use zecsplit_crypto::{Ask, Role};
use zecsplit_protocol::bob::{BobAction, BobInput, BobState, step_bob};
use zecsplit_protocol::machine::{AbortReason, Alert, ChainView, Params, SweepStatus, ZecLock};
use zecsplit_protocol::messages::{Envelope, Message, PROTOCOL_VERSION};
use zecsplit_protocol::recovery::{SecretString, open, seal};
use zecsplit_protocol::session::Shared;
use zecsplit_protocol::session::bob::BobSession;
use zecsplit_store::{NewSwap, Store, SwapRow};

#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    #[error("store: {0}")]
    Store(#[from] zecsplit_store::StoreError),
    #[error("session: {0}")]
    Session(#[from] zecsplit_protocol::session::SessionError),
    #[error("recovery bundle: {0}")]
    Recovery(#[from] zecsplit_protocol::recovery::RecoveryError),
    #[error("bitcoin source: {0}")]
    Source(#[from] zecsplit_btc::sources::SourceError),
    #[error("port: {0}")]
    Port(String),
    #[error("saved state: {0}")]
    Saved(String),
    #[error("§3.5-R5: tx_lock before Alice's cancel sig and refund adaptor sig")]
    LockTooEarly,
}

/// Bob's Bitcoin wallet, which funds and signs `tx_lock` (§3.4).
#[async_trait]
pub trait Wallet: Send + Sync {
    /// A fresh wallet address for the refund output.
    async fn refund_address(&self) -> Result<String, String>;
    /// An unsigned PSBT paying `value` to `spk` from wallet coins.
    async fn fund(&self, spk: &ScriptBuf, value: Amount) -> Result<Psbt, String>;
    /// `psbt` with every wallet input signed and finalized.
    async fn sign(&self, psbt: &Psbt) -> Result<Transaction, String>;
}

/// Bob's own Zebra node (ADR-009): lock detection and the redeem-path sweep (§4.3, §4.5-R3).
#[async_trait]
pub trait ZecNode: Send + Sync {
    /// §4.3-R1, §4.3-R2: the lock notes as this node sees them, counting `min_conf` confirmations.
    async fn lock(&self, shared: &Shared, zatoshis: u64, min_conf: u32) -> Result<ZecLock, String>;
    /// Sweeps every lock note with `ask`; returns the txid.
    async fn sweep(&self, shared: &Shared, ask: &Ask) -> Result<String, String>;
    /// Confirmations of a sweep, `None` while unmined.
    async fn confirmations(&self, shared: &Shared, txid: &str) -> Result<Option<u32>, String>;
}

/// Everything the runner talks to.
pub struct Ports<'a> {
    pub sources: &'a [&'a dyn BtcSource],
    pub broadcast: &'a dyn Broadcast,
    pub wallet: &'a dyn Wallet,
    pub zec: &'a dyn ZecNode,
}

/// One swap, driven from the maker side.
pub struct BobRunner {
    pub swap_id: [u8; 32],
    pub state: BobState,
    session: BobSession,
    params: Params,
    passphrase: SecretString,
    endpoint: String,
    outbox: VecDeque<Message>,
    pub alerts: Vec<Alert>,
    started_at: i64,
}

const ZEC_SWEEP: &str = "zec_sweep";
/// ADR-033: seconds a taker has to finish setup before Bob drops the swap with nothing locked.
pub const SETUP_TIMEOUT_SECS: i64 = 600;

fn json<T: serde::Serialize>(v: &T) -> Result<String, RunnerError> {
    serde_json::to_string(v).map_err(|e| RunnerError::Saved(e.to_string()))
}

impl BobRunner {
    /// Starts a swap for an accepted quote and records it before anything is answered.
    pub fn start(
        store: &mut Store,
        session: BobSession,
        passphrase: SecretString,
        endpoint: &str,
        now: i64,
    ) -> Result<Self, RunnerError> {
        let q = &session.core.quote;
        store.create_swap(
            &NewSwap {
                id: q.swap_id,
                role: Role::Bob,
                network: &json(&q.network)?,
                quote_json: &json(q)?,
                t1_height: u32::from(q.t1),
                t2_height: u32::from(q.t2),
                state_json: &json(&BobState::Quoted)?,
            },
            now,
        )?;
        Ok(BobRunner {
            swap_id: q.swap_id,
            state: BobState::Quoted,
            params: Params::from(q),
            session,
            passphrase,
            endpoint: endpoint.to_string(),
            outbox: VecDeque::new(),
            alerts: Vec::new(),
            started_at: now,
        })
    }

    /// §6.3-R2: rebuilds a stored swap; one stopped before the reveals is aborted instead.
    pub fn resume(
        store: &mut Store,
        row: &SwapRow,
        passphrase: SecretString,
        now: i64,
    ) -> Result<Option<Self>, RunnerError> {
        let state: BobState =
            serde_json::from_str(&row.state_json).map_err(|e| RunnerError::Saved(e.to_string()))?;
        let Some(sealed) = store.secret(&row.id)? else {
            let aborted = BobState::Aborted(AbortReason::NoLock);
            store.commit_transition(&row.id, &json(&aborted)?, true, "abort", "null", now)?;
            return Ok(None);
        };
        let bundle = open(&sealed, passphrase.clone())?;
        let endpoint = bundle.maker_endpoint.clone();
        let session = BobSession::restore(bundle)?;
        Ok(Some(BobRunner {
            swap_id: row.id,
            state,
            params: Params::from(&session.core.quote),
            session,
            passphrase,
            endpoint,
            outbox: VecDeque::new(),
            alerts: Vec::new(),
            started_at: row.created_at,
        }))
    }

    /// The next message for the taker, if any (INTERFACES §2 `GET …/next`).
    pub fn next_message(&mut self) -> Option<Envelope> {
        self.outbox.pop_front().map(|message| Envelope {
            v: PROTOCOL_VERSION,
            swap_id: self.swap_id,
            message,
        })
    }

    /// Seals the session into the `secrets` table once both reveals are in (ADR-039).
    fn save(&self, store: &mut Store) -> Result<(), RunnerError> {
        if self.session.core.shared_key().is_some() {
            let bundle = self.session.save(&self.endpoint)?;
            store.put_secret(&self.swap_id, &seal(&bundle, self.passphrase.clone())?)?;
        }
        Ok(())
    }

    /// Feeds inputs through the machine: persist the new state, then perform its actions (§6.3-R1).
    pub async fn feed(
        &mut self,
        store: &mut Store,
        ports: &Ports<'_>,
        input: BobInput,
        now: i64,
    ) -> Result<(), RunnerError> {
        let (next, actions) = step_bob(self.state, &input, &self.params);
        if next != self.state {
            self.save(store)?;
            store.commit_transition(
                &self.swap_id,
                &json(&next)?,
                next.is_terminal(),
                "step",
                &json(&format!("{input:?}"))?,
                now,
            )?;
            self.state = next;
        }
        for action in actions {
            self.act(store, ports, action, now).await?;
        }
        Ok(())
    }

    /// Handles one taker message (INTERFACES §2 `POST …/msg`); replies land in the outbox.
    pub async fn handle(
        &mut self,
        store: &mut Store,
        ports: &Ports<'_>,
        env: Envelope,
        now: i64,
    ) -> Result<(), RunnerError> {
        if let Some(input) = self.on_message(store, env)? {
            self.feed(store, ports, input, now).await?;
        }
        Ok(())
    }

    /// Nothing is on chain yet, so a stalled taker costs Bob nothing but a slot.
    pub fn in_setup(&self) -> bool {
        matches!(
            self.state,
            BobState::Quoted | BobState::Committed | BobState::Revealed
        )
    }

    /// One tick: the setup timer before the lock, a fresh chain snapshot after it.
    pub async fn tick(
        &mut self,
        store: &mut Store,
        ports: &Ports<'_>,
        now: i64,
    ) -> Result<(), RunnerError> {
        if self.in_setup() {
            if now - self.started_at >= SETUP_TIMEOUT_SECS {
                self.feed(store, ports, BobInput::SetupTimedOut, now)
                    .await?;
            }
            return Ok(());
        }
        let view = self.view(store, ports).await?;
        self.feed(store, ports, BobInput::Chain(view), now).await
    }

    /// Verifies one taker message into a machine input; anything invalid is misbehaviour.
    fn on_message(
        &mut self,
        store: &mut Store,
        env: Envelope,
    ) -> Result<Option<BobInput>, RunnerError> {
        if env.swap_id != self.swap_id {
            return Ok(Some(BobInput::PeerMisbehaved));
        }
        let core = &mut self.session.core;
        let verified = match &env.message {
            Message::Commit(c) => core.on_commit(c).map(|()| BobInput::PeerCommitted),
            Message::Reveal(r) => core
                .reveal_msg()
                .and_then(|_| core.on_reveal(r))
                .map(|_| BobInput::PeerRevealVerified),
            Message::SetupSigsAlice(s) => self
                .session
                .on_setup_sigs_alice(s)
                .map(|_| BobInput::PeerSigsVerified),
            Message::LockBroadcast(_) | Message::Status(_) => return Ok(None),
            Message::QuoteRequest(_)
            | Message::Quote(_)
            | Message::LockTemplate(_)
            | Message::SetupSigsBob(_)
            | Message::RedeemEncSig(_)
            | Message::Error(_) => return Ok(Some(BobInput::PeerMisbehaved)),
        };
        match verified {
            Ok(input) => {
                self.save(store)?;
                Ok(Some(input))
            }
            Err(_) => Ok(Some(BobInput::PeerMisbehaved)),
        }
    }

    async fn broadcast(
        &self,
        store: &mut Store,
        ports: &Ports<'_>,
        name: &str,
        tx: &Transaction,
        now: i64,
    ) -> Result<(), RunnerError> {
        let raw = bitcoin::consensus::encode::serialize(tx);
        store.put_tx(&self.swap_id, name, &raw, &tx.compute_txid().to_string())?;
        ports.broadcast.broadcast(tx).await?;
        store.mark_broadcast(&self.swap_id, name, now)?;
        Ok(())
    }

    async fn act(
        &mut self,
        store: &mut Store,
        ports: &Ports<'_>,
        action: BobAction,
        now: i64,
    ) -> Result<(), RunnerError> {
        match action {
            BobAction::SendCommit => {
                let body = self.session.core.commit_msg();
                self.outbox.push_back(Message::Commit(body));
            }
            BobAction::SendReveal => {
                let reveal = self.session.core.reveal_msg()?;
                self.outbox.push_back(Message::Reveal(Box::new(reveal)));
            }
            BobAction::SendLockTemplate => {
                let tpl = self.lock_template(store, ports).await?;
                self.outbox.push_back(Message::LockTemplate(tpl));
            }
            BobAction::SendSetupSigs => {
                let sigs = self.session.setup_sigs()?;
                self.outbox.push_back(Message::SetupSigsBob(sigs));
            }
            BobAction::BroadcastLock => self.broadcast_lock(store, ports, now).await?,
            BobAction::SendEncSig => {
                let esig = self.session.encsig()?;
                self.save(store)?;
                self.outbox.push_back(Message::RedeemEncSig(esig));
            }
            BobAction::SweepZec => self.sweep(store, ports).await?,
            BobAction::BroadcastCancel => {
                let tx = self.session.signed_cancel()?;
                self.broadcast(store, ports, "tx_cancel", &tx, now).await?;
            }
            BobAction::BroadcastRefund => {
                let tx = self.session.signed_refund()?;
                self.broadcast(store, ports, "tx_refund", &tx, now).await?;
            }
            BobAction::CpfpRefund => {}
            BobAction::Alert(a) => {
                if !self.alerts.contains(&a) {
                    self.alerts.push(a);
                }
            }
        }
        Ok(())
    }

    /// §3.5-R1: the template already sent, or a freshly funded one saved before it leaves.
    async fn lock_template(
        &mut self,
        store: &mut Store,
        ports: &Ports<'_>,
    ) -> Result<zecsplit_protocol::messages::LockTemplate, RunnerError> {
        if let Some(tpl) = self.session.lock_template_msg()? {
            return Ok(tpl);
        }
        let refund = ports
            .wallet
            .refund_address()
            .await
            .map_err(RunnerError::Port)?;
        let (spk, v_lock) = self.session.lock_output(&refund)?;
        let psbt = ports
            .wallet
            .fund(&spk, v_lock)
            .await
            .map_err(RunnerError::Port)?;
        let tpl = self.session.lock_template(psbt, &refund)?;
        self.save(store)?;
        Ok(tpl)
    }

    /// §3.5-R5: signs and broadcasts `tx_lock`, only once Alice's setup sigs are verified.
    async fn broadcast_lock(
        &self,
        store: &mut Store,
        ports: &Ports<'_>,
        now: i64,
    ) -> Result<(), RunnerError> {
        if !self.session.may_broadcast_lock() {
            return Err(RunnerError::LockTooEarly);
        }
        let psbt = &self
            .session
            .templates()
            .ok_or(RunnerError::LockTooEarly)?
            .lock;
        let tx = ports.wallet.sign(psbt).await.map_err(RunnerError::Port)?;
        if tx.compute_txid() != psbt.unsigned_tx.compute_txid() {
            return Err(RunnerError::Port(
                "wallet changed tx_lock while signing".into(),
            ));
        }
        self.broadcast(store, ports, "tx_lock", &tx, now).await
    }

    /// §4.5-R3: recovers `s_a` from Alice's published redeem and sweeps the ZEC, once.
    async fn sweep(&self, store: &mut Store, ports: &Ports<'_>) -> Result<(), RunnerError> {
        if store
            .txs(&self.swap_id)?
            .iter()
            .any(|t| t.name == ZEC_SWEEP)
        {
            return Ok(());
        }
        let Some(templates) = self.session.templates() else {
            return Ok(());
        };
        let Some(redeem) = fetch_any(ports.sources, &templates.redeem.compute_txid()).await? else {
            return Ok(());
        };
        let ask = self.session.ask_from_redeem(&redeem)?;
        let shared = self
            .session
            .core
            .shared_key()
            .ok_or_else(|| RunnerError::Saved("no shared key".into()))?;
        let txid = ports
            .zec
            .sweep(&shared, &ask)
            .await
            .map_err(RunnerError::Port)?;
        store.put_tx(&self.swap_id, ZEC_SWEEP, txid.as_bytes(), &txid)?;
        Ok(())
    }

    /// §4.2-R1 for Bitcoin; Bob's own node for Zcash (ADR-009).
    async fn view(&self, store: &mut Store, ports: &Ports<'_>) -> Result<ChainView, RunnerError> {
        let mut view = match self.session.templates() {
            Some(t) => ChainView::from(swap_status(ports.sources, t).await?),
            None => ChainView::at_tip(tip_on_all(ports.sources).await?),
        };
        if let Some(shared) = self.session.core.shared_key() {
            view.zec_lock = ports
                .zec
                .lock(
                    &shared,
                    self.session.core.quote.zec_amount,
                    u32::from(self.params.n_conf_zec),
                )
                .await
                .map_err(RunnerError::Port)?;
        }
        if let Some(sweep) = store
            .txs(&self.swap_id)?
            .into_iter()
            .find(|t| t.name == ZEC_SWEEP)
            && let Some(shared) = self.session.core.shared_key()
        {
            view.zec_sweep = match ports
                .zec
                .confirmations(&shared, &sweep.txid)
                .await
                .map_err(RunnerError::Port)?
            {
                Some(confirmations) => SweepStatus::Confirmed { confirmations },
                None => SweepStatus::Mempool,
            };
        }
        Ok(view)
    }
}
