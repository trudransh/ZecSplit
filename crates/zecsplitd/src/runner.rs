//! Alice's swap runner: steps her state machine on chain snapshots, persisting before acting (§6.3, ADR-033).

use std::collections::VecDeque;

use async_trait::async_trait;
use bitcoin::Transaction;
use zecsplit_btc::sources::{Broadcast, BtcSource, fetch_any, swap_status, tip_on_all};
use zecsplit_crypto::{Ask, Role};
use zecsplit_protocol::alice::{AliceAction, AliceInput, AliceState, step_alice};
use zecsplit_protocol::machine::{AbortReason, Alert, ChainView, Params, SweepStatus};
use zecsplit_protocol::messages::{Envelope, Message, Network, PROTOCOL_VERSION};
use zecsplit_protocol::recovery::{SecretString, open, seal};
use zecsplit_protocol::session::Shared;
use zecsplit_protocol::session::alice::AliceSession;
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
}

/// The maker endpoint (INTERFACES §2 routes `POST …/msg` and `GET …/next`).
#[async_trait]
pub trait Maker: Send + Sync {
    /// Delivers one message and returns the maker's reply, if any.
    async fn send(&self, msg: &Envelope) -> Result<Option<Envelope>, String>;
    /// The maker's next pending message for `swap_id`, if any.
    async fn next(&self, swap_id: &[u8; 32]) -> Result<Option<Envelope>, String>;
}

/// The Zcash side Alice needs: sweeping the lock back on the refund path (§5.3-R3).
#[async_trait]
pub trait ZecSweep: Send + Sync {
    /// Sweeps every lock note with `ask`; returns the txid.
    async fn sweep(&self, shared: &Shared, ask: &Ask) -> Result<String, String>;
    /// Confirmations of a sweep, `None` while unmined.
    async fn confirmations(&self, shared: &Shared, txid: &str) -> Result<Option<u32>, String>;
}

/// Everything the runner talks to.
pub struct Ports<'a> {
    pub maker: &'a dyn Maker,
    pub sources: &'a [&'a dyn BtcSource],
    pub broadcast: &'a dyn Broadcast,
    pub zec: &'a dyn ZecSweep,
}

/// One swap, driven from the taker side.
pub struct AliceRunner {
    pub swap_id: [u8; 32],
    pub state: AliceState,
    session: AliceSession,
    params: Params,
    passphrase: SecretString,
    maker_endpoint: String,
    /// `(lock UA, zatoshis)` while the UI may show the payment request (§4.2-R2).
    pub payment_request: Option<(String, u64)>,
    pub alerts: Vec<Alert>,
}

const ZEC_SWEEP: &str = "zec_sweep";
const MAX_MESSAGES_PER_TICK: usize = 8;

fn json<T: serde::Serialize>(v: &T) -> Result<String, RunnerError> {
    serde_json::to_string(v).map_err(|e| RunnerError::Saved(e.to_string()))
}

/// §3.3-R4: the lock address for this swap's Zcash network.
fn lock_ua(network: Network, s: &Shared) -> Result<String, RunnerError> {
    let keys = match network {
        Network::Regtest => {
            zecsplit_zcash::lock_keys(&zecsplit_zcash::regtest(), &s.ak, &s.nk, &s.rivk)
        }
        Network::Test => zecsplit_zcash::lock_keys(
            &zcash_protocol::consensus::Network::TestNetwork,
            &s.ak,
            &s.nk,
            &s.rivk,
        ),
        Network::MainBeta => return Err(RunnerError::Saved("mainnet is cut (ADR-025)".into())),
    };
    keys.map(|k| k.ua)
        .map_err(|e| RunnerError::Saved(e.to_string()))
}

impl AliceRunner {
    /// Starts a new swap from a verified quote and records it before anything is sent.
    pub fn start(
        store: &mut Store,
        session: AliceSession,
        passphrase: SecretString,
        maker_endpoint: &str,
        now: i64,
    ) -> Result<Self, RunnerError> {
        let q = &session.core.quote;
        store.create_swap(
            &NewSwap {
                id: q.swap_id,
                role: Role::Alice,
                network: &json(&q.network)?,
                quote_json: &json(q)?,
                t1_height: u32::from(q.t1),
                t2_height: u32::from(q.t2),
                state_json: &json(&AliceState::Quoted)?,
            },
            now,
        )?;
        Ok(AliceRunner {
            swap_id: q.swap_id,
            state: AliceState::Quoted,
            params: Params::from(q),
            session,
            passphrase,
            maker_endpoint: maker_endpoint.to_string(),
            payment_request: None,
            alerts: Vec::new(),
        })
    }

    /// §6.3-R2: rebuilds a stored swap; one stopped before setup completed is aborted instead.
    pub fn resume(
        store: &mut Store,
        row: &SwapRow,
        passphrase: SecretString,
        now: i64,
    ) -> Result<Option<Self>, RunnerError> {
        let state: AliceState =
            serde_json::from_str(&row.state_json).map_err(|e| RunnerError::Saved(e.to_string()))?;
        let Some(sealed) = store.secret(&row.id)? else {
            let aborted = AliceState::Aborted(AbortReason::NoLock);
            store.commit_transition(&row.id, &json(&aborted)?, true, "abort", "null", now)?;
            return Ok(None);
        };
        let bundle = open(&sealed, passphrase.clone())?;
        let maker_endpoint = bundle.maker_endpoint.clone();
        let session = AliceSession::restore(bundle)?;
        Ok(Some(AliceRunner {
            swap_id: row.id,
            state,
            params: Params::from(&session.core.quote),
            session,
            passphrase,
            maker_endpoint,
            payment_request: None,
            alerts: Vec::new(),
        }))
    }

    fn envelope(&self, message: Message) -> Envelope {
        Envelope {
            v: PROTOCOL_VERSION,
            swap_id: self.swap_id,
            message,
        }
    }

    /// Seals the session into the `secrets` table once it holds every §6.4-R1 blob (ADR-039).
    fn save(&self, store: &mut Store) -> Result<(), RunnerError> {
        if self.session.is_complete() {
            let bundle = self.session.save(&self.maker_endpoint)?;
            store.put_secret(&self.swap_id, &seal(&bundle, self.passphrase.clone())?)?;
        }
        Ok(())
    }

    /// Feeds inputs through the machine: persist the new state, then perform its actions (§6.3-R1).
    pub async fn feed(
        &mut self,
        store: &mut Store,
        ports: &Ports<'_>,
        first: AliceInput,
        now: i64,
    ) -> Result<(), RunnerError> {
        let mut queue = VecDeque::from([first]);
        while let Some(input) = queue.pop_front() {
            let (next, actions) = step_alice(self.state, &input, &self.params);
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
                if let Some(follow) = self.act(store, ports, action, now).await? {
                    queue.push_back(follow);
                }
            }
        }
        Ok(())
    }

    /// One tick: deliver pending maker messages, then step on a fresh chain snapshot.
    pub async fn tick(
        &mut self,
        store: &mut Store,
        ports: &Ports<'_>,
        now: i64,
    ) -> Result<(), RunnerError> {
        for _ in 0..MAX_MESSAGES_PER_TICK {
            if self.state.is_terminal() {
                break;
            }
            let Some(env) = ports
                .maker
                .next(&self.swap_id)
                .await
                .map_err(RunnerError::Port)?
            else {
                break;
            };
            if let Some(input) = self.on_message(env, store)? {
                self.feed(store, ports, input, now).await?;
            }
        }
        let view = self.view(store, ports).await?;
        self.feed(store, ports, AliceInput::Chain(view), now).await
    }

    /// Verifies one maker message into a machine input; anything invalid is misbehaviour.
    fn on_message(
        &mut self,
        env: Envelope,
        store: &mut Store,
    ) -> Result<Option<AliceInput>, RunnerError> {
        if env.swap_id != self.swap_id {
            return Ok(Some(AliceInput::PeerMisbehaved));
        }
        let verified = match &env.message {
            Message::Commit(c) => self
                .session
                .core
                .on_commit(c)
                .map(|()| AliceInput::PeerCommitted),
            Message::Reveal(r) => self
                .session
                .core
                .on_reveal(r)
                .map(|_| AliceInput::PeerRevealVerified),
            Message::LockTemplate(t) => self
                .session
                .on_lock_template(t)
                .map(|_| AliceInput::LockTemplateVerified),
            Message::SetupSigsBob(s) => self
                .session
                .on_setup_sigs_bob(s)
                .map(|()| AliceInput::PeerSigsVerified),
            Message::RedeemEncSig(e) => self
                .session
                .on_encsig(e)
                .map(|()| AliceInput::EncSigVerified),
            Message::LockBroadcast(_) | Message::Status(_) => return Ok(None),
            Message::QuoteRequest(_)
            | Message::Quote(_)
            | Message::SetupSigsAlice(_)
            | Message::Error(_) => return Ok(Some(AliceInput::PeerMisbehaved)),
        };
        match verified {
            Ok(input) => {
                self.save(store)?;
                Ok(Some(input))
            }
            Err(_) => Ok(Some(AliceInput::PeerMisbehaved)),
        }
    }

    async fn send(
        &mut self,
        store: &mut Store,
        ports: &Ports<'_>,
        message: Message,
    ) -> Result<Option<AliceInput>, RunnerError> {
        let reply = ports
            .maker
            .send(&self.envelope(message))
            .await
            .map_err(RunnerError::Port)?;
        match reply {
            Some(env) => self.on_message(env, store),
            None => Ok(None),
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
        action: AliceAction,
        now: i64,
    ) -> Result<Option<AliceInput>, RunnerError> {
        match action {
            AliceAction::SendCommit => {
                let body = self.session.core.commit_msg();
                self.send(store, ports, Message::Commit(body)).await
            }
            AliceAction::SendReveal => {
                let reveal = self.session.core.reveal_msg()?;
                self.send(store, ports, Message::Reveal(Box::new(reveal)))
                    .await
            }
            AliceAction::SendSetupSigs => {
                let sigs = self.session.setup_sigs()?;
                self.send(store, ports, Message::SetupSigsAlice(sigs)).await
            }
            AliceAction::ShowPaymentRequest => {
                let shared = self
                    .session
                    .core
                    .shared_key()
                    .ok_or_else(|| RunnerError::Saved("no shared key".into()))?;
                let ua = lock_ua(self.session.core.quote.network, &shared)?;
                self.payment_request = Some((ua, self.session.core.quote.zec_amount));
                Ok(None)
            }
            AliceAction::HidePaymentRequest => {
                self.payment_request = None;
                Ok(None)
            }
            AliceAction::BroadcastRedeem => {
                let tx = self.session.signed_redeem()?;
                self.broadcast(store, ports, "tx_redeem", &tx, now).await?;
                Ok(None)
            }
            AliceAction::BroadcastCancel => {
                let tx = self.session.signed_cancel()?;
                self.broadcast(store, ports, "tx_cancel", &tx, now).await?;
                Ok(None)
            }
            AliceAction::BroadcastPunish => {
                let tx = self.session.signed_punish()?;
                self.broadcast(store, ports, "tx_punish", &tx, now).await?;
                Ok(None)
            }
            AliceAction::SweepZec => {
                self.sweep(store, ports).await?;
                Ok(None)
            }
            AliceAction::CpfpRedeem => Ok(None),
            AliceAction::Alert(a) => {
                if !self.alerts.contains(&a) {
                    self.alerts.push(a);
                }
                Ok(None)
            }
        }
    }

    /// §5.3-R3: recovers `s_b` from Bob's published refund and sweeps the ZEC back, once.
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
        let refund_txid = templates.refund.compute_txid();
        let Some(published) = fetch_any(ports.sources, &refund_txid).await? else {
            return Ok(());
        };
        let ask = self.session.ask_from_refund(&published)?;
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

    /// §4.2-R1: every Bitcoin status comes from all sources agreeing.
    async fn view(&self, store: &mut Store, ports: &Ports<'_>) -> Result<ChainView, RunnerError> {
        let mut view = match self.session.templates() {
            Some(t) => ChainView::from(swap_status(ports.sources, t).await?),
            None => ChainView::at_tip(tip_on_all(ports.sources).await?),
        };
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

#[cfg(test)]
#[path = "runner_tests.rs"]
mod tests;
