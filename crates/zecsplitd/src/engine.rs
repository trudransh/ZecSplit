//! The taker engine behind INTERFACES §3: quotes, swaps, the view model and recovery files.

use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::http::StatusCode;
use bitcoin::base64::Engine as _;
use bitcoin::base64::engine::general_purpose::STANDARD as BASE64;
use zecsplit_crypto::Role;
use zecsplit_crypto::rand_core::OsRng;
use zecsplit_protocol::alice::{AliceInput, AliceState};
use zecsplit_protocol::machine::{
    AbortReason, Alert, ChainView, Outcome as End, TxStatus, check_terms,
};
use zecsplit_protocol::messages::{self as wire, Quote};
use zecsplit_protocol::recovery::{SecretString, open, seal};
use zecsplit_protocol::session::alice::AliceSession;
use zecsplit_protocol::session::{Core, btc_network};
use zecsplit_store::{NewSwap, Store, SwapRow, TxRow};

use crate::api::{
    ApiError, Confirmations, CreateSwapRequest, CreateSwapResponse, Deadline, ErrorCode, Health,
    Network, Outcome, QuoteView, RestoreRequest, Sats, Step, StepKind, StepStatus, SwapSummary,
    SwapView, Window, Zatoshis,
};
use crate::maker_client::HttpMaker;
use crate::runner::{AliceRunner, Ports};

/// UX_SPEC §2.4: the passphrase also encrypts the recovery file.
const MIN_PASSPHRASE: usize = 10;
/// Display estimates only; deadlines themselves are heights (QUALITY_SYSTEM §2.7).
const BLOCK_SECS: u64 = 600;

/// What the user configures.
pub struct Config {
    pub network: wire::Network,
    /// The maker's base URL, also written into each recovery file.
    pub maker_url: String,
    /// §2-R1: the maker key quotes must be signed with.
    pub maker_key: [u8; 32],
}

/// A refused request: the HTTP status and the INTERFACES §3 error body.
#[derive(Debug)]
pub struct Refusal {
    pub status: StatusCode,
    pub body: ApiError,
}

fn refuse(status: StatusCode, code: &str, message: impl std::fmt::Display) -> Refusal {
    Refusal {
        status,
        body: ApiError {
            code: code.to_string(),
            message: message.to_string(),
        },
    }
}

fn internal(e: impl std::fmt::Display) -> Refusal {
    refuse(StatusCode::INTERNAL_SERVER_ERROR, "internal", e)
}

fn locked() -> Refusal {
    refuse(
        StatusCode::UNAUTHORIZED,
        "locked",
        "unlock the engine first",
    )
}

fn unknown_swap() -> Refusal {
    refuse(StatusCode::NOT_FOUND, "unknown_swap", "no such swap")
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

fn unix() -> u64 {
    u64::try_from(now()).unwrap_or(0)
}

fn swap_id(hex_id: &str) -> Result<[u8; 32], Refusal> {
    let mut id = [0; 32];
    hex::decode_to_slice(hex_id, &mut id).map_err(|_| unknown_swap())?;
    Ok(id)
}

fn api_network(n: wire::Network) -> Network {
    match n {
        wire::Network::Regtest => Network::Regtest,
        wire::Network::Test => Network::Test,
        wire::Network::MainBeta => Network::MainBeta,
    }
}

/// When block `height` is expected, seen from `tip` at `now`.
fn estimate(now: u64, tip: u32, height: u32) -> u64 {
    now + u64::from(height.saturating_sub(tip)) * BLOCK_SECS
}

/// ZIP 321 for one payment; the amount in ZEC with no trailing zeros.
fn zip321(ua: &str, zats: u64) -> String {
    let frac = format!("{:08}", zats % 100_000_000);
    let frac = frac.trim_end_matches('0');
    let amount = if frac.is_empty() {
        format!("{}", zats / 100_000_000)
    } else {
        format!("{}.{frac}", zats / 100_000_000)
    };
    format!("zcash:{ua}?amount={amount}")
}

/// UX_SPEC §2.5 step statuses per state, never green before it's true (§1.3). Off the happy
/// path the engine can't tell whether the user paid until a sweep finds the ZEC.
fn statuses(state: AliceState, zec_swept: bool) -> [StepStatus; 6] {
    use AliceState as S;
    use StepStatus::{Confirmed as C, Failed as F, Pending as P, Seen, Skipped as K, Waiting as W};
    let at = |done: usize, current: StepStatus| {
        let mut s = [P; 6];
        s[..done].fill(C);
        s[done] = current;
        s
    };
    let paid = if zec_swept { C } else { K };
    match state {
        S::Quoted | S::Committed | S::Revealed { .. } => at(0, W),
        S::SigsExchanged => at(1, W),
        S::BtcLockSeen => at(1, Seen),
        S::AwaitingZecPayment { .. } => at(2, W),
        S::EncSigReceived { .. } => at(4, W),
        S::RedeemBroadcast { .. } => at(4, Seen),
        S::Done(End::Swapped) => [C; 6],
        S::CancelBroadcast { .. }
        | S::Cancelled { .. }
        | S::RefundSeen
        | S::ZecSweepBroadcast
        | S::PunishBroadcast { .. } => [C, C, paid, F, K, W],
        S::Done(End::Refunded | End::Punished) => [C, C, paid, F, K, C],
        S::Aborted(AbortReason::NoLock) => [C, F, K, K, K, K],
        S::Aborted(AbortReason::PeerMisbehaved) => [F, K, K, K, K, K],
    }
}

fn outcome(state: AliceState) -> Option<Outcome> {
    match state {
        AliceState::Done(End::Swapped) => Some(Outcome::Redeemed),
        AliceState::Done(End::Refunded) => Some(Outcome::Refunded),
        AliceState::Done(End::Punished) => Some(Outcome::Punished),
        AliceState::Aborted(_) => Some(Outcome::Aborted),
        _ => None,
    }
}

/// UX_SPEC §3: the latest alert, else what the state itself says.
fn error_code(state: AliceState, alerts: &[Alert]) -> Option<ErrorCode> {
    use AliceState as S;
    match alerts.last() {
        Some(Alert::BtcSourcesDisagree) => return Some(ErrorCode::BtcSourcesDisagree),
        Some(Alert::MarginTooSmall) => return Some(ErrorCode::MarginTooSmall),
        Some(Alert::PeerMisbehavedAfterLock) | None => {}
    }
    match state {
        S::Aborted(AbortReason::NoLock) => Some(ErrorCode::MakerNoLock),
        S::CancelBroadcast { .. }
        | S::Cancelled { .. }
        | S::RefundSeen
        | S::ZecSweepBroadcast
        | S::PunishBroadcast { .. }
        | S::Done(End::Refunded | End::Punished) => Some(ErrorCode::MakerSilentAfterZec),
        _ => None,
    }
}

fn h_lock(state: AliceState) -> Option<u32> {
    match state {
        AliceState::AwaitingZecPayment { h_lock }
        | AliceState::EncSigReceived { h_lock }
        | AliceState::RedeemBroadcast { h_lock, .. }
        | AliceState::CancelBroadcast { h_lock } => Some(h_lock),
        _ => None,
    }
}

/// After a restart finished swaps have no runner; the stored redeem still names the payout.
fn redeem_address(txs: &[TxRow], network: wire::Network) -> Option<String> {
    let raw = &txs.iter().find(|t| t.name == "tx_redeem")?.raw;
    let tx: bitcoin::Transaction = bitcoin::consensus::deserialize(raw).ok()?;
    let spk = &tx.output.first()?.script_pubkey;
    let address = bitcoin::Address::from_script(spk, btc_network(network)).ok()?;
    Some(address.to_string())
}

fn confirmations(status: TxStatus, tip: u32, need: u16) -> Option<Confirmations> {
    let have = match status {
        TxStatus::Mempool => 0,
        TxStatus::Confirmed { height } => tip.saturating_sub(height) + 1,
        TxStatus::Unseen | TxStatus::SourcesDisagree => return None,
    };
    Some(Confirmations {
        have,
        need: u32::from(need),
    })
}

/// The taker side of every swap in one store.
pub struct Engine {
    config: Config,
    store: Store,
    maker: &'static HttpMaker,
    ports: Ports<'static>,
    passphrase: Option<SecretString>,
    runners: HashMap<[u8; 32], AliceRunner>,
    quotes: HashMap<[u8; 32], Quote>,
    // ponytail: in memory, so after a restart the UI asks again; a column if that annoys.
    recovery_saved: HashSet<[u8; 32]>,
    /// Restored swaps never show a payment request: the user may have paid already.
    restored: HashSet<[u8; 32]>,
}

impl Engine {
    /// Opens the engine locked; swaps resume on `unlock`.
    pub fn new(
        config: Config,
        store: Store,
        maker: &'static HttpMaker,
        ports: Ports<'static>,
    ) -> Self {
        Engine {
            config,
            store,
            maker,
            ports,
            passphrase: None,
            runners: HashMap::new(),
            quotes: HashMap::new(),
            recovery_saved: HashSet::new(),
            restored: HashSet::new(),
        }
    }

    /// `GET /api/health`.
    pub fn health(&self) -> Health {
        Health {
            version: env!("CARGO_PKG_VERSION").to_string(),
            network: api_network(self.config.network),
            unlocked: self.passphrase.is_some(),
        }
    }

    /// `POST /api/unlock`: checks the passphrase against a sealed swap, then resumes every active
    /// swap (§6.3-R2).
    pub async fn unlock(&mut self, passphrase: &str) -> Result<(), Refusal> {
        if passphrase.chars().count() < MIN_PASSPHRASE {
            return Err(refuse(
                StatusCode::BAD_REQUEST,
                "weak_passphrase",
                format!("use at least {MIN_PASSPHRASE} characters"),
            ));
        }
        let pass = SecretString::from(passphrase.to_owned());
        for row in self.store.swaps().map_err(internal)? {
            if let Some(sealed) = self.store.secret(&row.id).map_err(internal)? {
                open(&sealed, pass.clone()).map_err(|_| {
                    refuse(
                        StatusCode::UNAUTHORIZED,
                        "wrong_passphrase",
                        "wrong passphrase",
                    )
                })?;
                break;
            }
        }
        if self.passphrase.is_some() {
            return Ok(());
        }
        let now = now();
        for row in self.store.active_swaps().map_err(internal)? {
            if row.role != Role::Alice {
                continue;
            }
            let resumed =
                AliceRunner::resume(&mut self.store, &row, pass.clone(), now).map_err(internal)?;
            if let Some(mut runner) = resumed {
                if let Err(e) = runner
                    .feed(&mut self.store, &self.ports, AliceInput::Resume, now)
                    .await
                {
                    eprintln!("resume {}: {e}", hex::encode(row.id));
                }
                self.runners.insert(row.id, runner);
            }
        }
        self.passphrase = Some(pass);
        Ok(())
    }

    /// `POST /api/quote`: a quote from the maker, verified against the pinned key (§2-R2).
    pub async fn quote(&mut self, zec_amount: u64) -> Result<QuoteView, Refusal> {
        let maker = |e: String| refuse(StatusCode::BAD_GATEWAY, "maker", e);
        let now = unix();
        self.quotes.retain(|_, q| q.expires_at > now);
        let info = self.maker.info().await.map_err(maker)?;
        let signed = self
            .maker
            .quote(zec_amount, self.config.network)
            .await
            .map_err(maker)?;
        let quote = signed
            .verify(&self.config.maker_key, now)
            .map_err(|e| refuse(StatusCode::BAD_GATEWAY, "bad_quote", e))?
            .clone();
        if quote.network != self.config.network || quote.zec_amount != zec_amount {
            return Err(refuse(
                StatusCode::BAD_GATEWAY,
                "bad_quote",
                "not the quote asked for",
            ));
        }
        check_terms(&quote).map_err(|e| refuse(StatusCode::BAD_GATEWAY, "bad_quote", e))?;
        let opens = estimate(now, 0, u32::from(quote.n_conf_btc) + u32::from(quote.t1));
        let view = QuoteView {
            quote_id: hex::encode(quote.swap_id),
            zec_amount: Zatoshis(quote.zec_amount),
            btc_amount: Sats(quote.btc_amount),
            expires_at: quote.expires_at,
            min_zec: Zatoshis(info.min_zec),
            max_zec: Zatoshis(info.max_zec),
            refund_window: Window {
                opens_at: opens,
                closes_at: opens + u64::from(quote.t2) * BLOCK_SECS,
            },
        };
        self.quotes.insert(quote.swap_id, quote);
        Ok(view)
    }

    /// `POST /api/swaps`: starts the swap for a quote this engine fetched.
    pub async fn create_swap(
        &mut self,
        req: &CreateSwapRequest,
    ) -> Result<CreateSwapResponse, Refusal> {
        let pass = self.passphrase.clone().ok_or_else(locked)?;
        if !req.accept_risks {
            return Err(refuse(
                StatusCode::BAD_REQUEST,
                "risks_not_accepted",
                "accept the risks first (UX_SPEC §2.3)",
            ));
        }
        let address = bitcoin::Address::from_str(&req.btc_address)
            .ok()
            .and_then(|a| a.require_network(btc_network(self.config.network)).ok())
            .ok_or_else(|| {
                refuse(
                    StatusCode::BAD_REQUEST,
                    "bad_btc_address",
                    "not a Bitcoin address on this network",
                )
            })?;
        let id = swap_id(&req.quote_id)?;
        let quote = self
            .quotes
            .remove(&id)
            .filter(|q| q.expires_at > unix())
            .ok_or_else(|| {
                refuse(
                    StatusCode::BAD_REQUEST,
                    "quote_expired",
                    "no such quote, or it expired",
                )
            })?;
        let core =
            Core::new(&mut OsRng, quote, Role::Alice, address.to_string()).map_err(internal)?;
        let now = now();
        let mut runner = AliceRunner::start(
            &mut self.store,
            AliceSession::new(core),
            pass,
            &self.config.maker_url,
            now,
        )
        .map_err(internal)?;
        let begun = runner
            .feed(&mut self.store, &self.ports, AliceInput::Begin, now)
            .await;
        self.runners.insert(id, runner);
        begun.map_err(|e| refuse(StatusCode::BAD_GATEWAY, "maker", e))?;
        Ok(CreateSwapResponse {
            swap_id: req.quote_id.clone(),
        })
    }

    /// One tick of every unfinished swap; a failing swap is reported and the others still run.
    pub async fn tick(&mut self) -> Vec<String> {
        let mut failures = Vec::new();
        for (id, runner) in self.runners.iter_mut() {
            if runner.state.is_terminal() {
                continue;
            }
            if let Err(e) = runner.tick(&mut self.store, &self.ports, now()).await {
                failures.push(format!("{}: {e}", hex::encode(id)));
            }
        }
        failures
    }

    fn row(&self, hex_id: &str) -> Result<SwapRow, Refusal> {
        let id = swap_id(hex_id)?;
        self.store
            .swaps()
            .map_err(internal)?
            .into_iter()
            .find(|r| r.id == id && r.role == Role::Alice)
            .ok_or_else(unknown_swap)
    }

    /// `GET /api/swaps`.
    pub fn swaps(&self) -> Result<Vec<SwapSummary>, Refusal> {
        let rows = self.store.swaps().map_err(internal)?;
        rows.iter()
            .filter(|r| r.role == Role::Alice)
            .map(|row| {
                let view = self.view(row)?;
                let current_step = view
                    .steps
                    .iter()
                    .find(|s| !matches!(s.status, StepStatus::Confirmed | StepStatus::Skipped))
                    .map_or(StepKind::Done, |s| s.kind);
                Ok(SwapSummary {
                    swap_id: view.swap_id,
                    created_at: u64::try_from(row.created_at).unwrap_or(0),
                    zec_amount: view.zec_amount,
                    btc_amount: view.btc_amount,
                    current_step,
                    outcome: view.outcome,
                })
            })
            .collect()
    }

    /// `GET /api/swaps/{id}` and each SSE event.
    pub fn swap(&self, hex_id: &str) -> Result<SwapView, Refusal> {
        self.view(&self.row(hex_id)?)
    }

    fn view(&self, row: &SwapRow) -> Result<SwapView, Refusal> {
        let quote: Quote = serde_json::from_str(&row.quote_json).map_err(internal)?;
        let state: AliceState = serde_json::from_str(&row.state_json).map_err(internal)?;
        let txs = self.store.txs(&row.id).map_err(internal)?;
        let txid = |name: &str| txs.iter().find(|t| t.name == name).map(|t| t.txid.clone());
        let live = self.runners.get(&row.id);
        let chain: Option<ChainView> = live.and_then(|r| r.last_view);
        let now = unix();
        let tip = chain.map(|c| c.btc_tip);
        let h_lock = h_lock(state);

        let at = |height: u32| Deadline {
            height,
            estimated_at: estimate(now, tip.unwrap_or(height), height),
        };
        let last_redeem =
            h_lock.map(|h| at(h + u32::from(quote.t1) - u32::from(quote.redeem_margin)));
        let refund_window = match h_lock {
            Some(h) => Window {
                opens_at: at(h + u32::from(quote.t1)).estimated_at,
                closes_at: at(h + u32::from(quote.t1) + u32::from(quote.t2)).estimated_at,
            },
            None => {
                let created = u64::try_from(row.created_at).unwrap_or(0);
                let opens = estimate(
                    created,
                    0,
                    u32::from(quote.n_conf_btc) + u32::from(quote.t1),
                );
                Window {
                    opens_at: opens,
                    closes_at: opens + u64::from(quote.t2) * BLOCK_SECS,
                }
            }
        };

        let kinds = [
            StepKind::Setup,
            StepKind::MakerLocksBtc,
            StepKind::PayZec,
            StepKind::MakerVerifies,
            StepKind::ClaimBtc,
            StepKind::Done,
        ];
        let statuses = statuses(state, txid("zec_sweep").is_some());
        let steps = kinds
            .into_iter()
            .zip(statuses)
            .map(|(kind, status)| {
                let shown = status != StepStatus::Pending && status != StepStatus::Skipped;
                let mut step = Step {
                    kind,
                    status,
                    confirmations: None,
                    txid: None,
                    deadline: None,
                };
                match kind {
                    StepKind::MakerLocksBtc if shown => {
                        step.txid = live.and_then(AliceRunner::lock_txid);
                        step.confirmations =
                            chain.and_then(|c| confirmations(c.lock, c.btc_tip, quote.n_conf_btc));
                    }
                    StepKind::PayZec if status == StepStatus::Waiting => {
                        step.deadline = last_redeem
                    }
                    StepKind::ClaimBtc if shown => {
                        step.txid = txid("tx_redeem");
                        step.confirmations = chain
                            .and_then(|c| confirmations(c.redeem, c.btc_tip, quote.n_conf_btc));
                        step.deadline = last_redeem;
                    }
                    _ => {}
                }
                step
            })
            .collect();

        let zip321_uri = live
            .and_then(|r| r.payment_request.as_ref())
            .filter(|_| self.recovery_saved.contains(&row.id) && !self.restored.contains(&row.id))
            .map(|(ua, zats)| zip321(ua, *zats));
        let core = live.map(|r| r.alerts.as_slice()).unwrap_or_default();
        Ok(SwapView {
            swap_id: hex::encode(row.id),
            network: api_network(quote.network),
            zec_amount: Zatoshis(quote.zec_amount),
            btc_amount: Sats(quote.btc_amount),
            btc_address: live
                .map(|r| r.payout_addr().to_string())
                .or_else(|| redeem_address(&txs, quote.network))
                .unwrap_or_default(),
            steps,
            zip321_uri,
            refund_window,
            recovery_saved: self.recovery_saved.contains(&row.id),
            outcome: outcome(state),
            error: error_code(state, core),
        })
    }

    /// `GET /api/swaps/{id}/recovery-file`: the sealed session as text (UX_SPEC §2.4 "Copy as
    /// text"); it exists once setup completes (ADR-040).
    pub fn recovery_file(&self, hex_id: &str) -> Result<String, Refusal> {
        let row = self.row(hex_id)?;
        let sealed = self
            .store
            .secret(&row.id)
            .map_err(internal)?
            .ok_or_else(|| {
                refuse(
                    StatusCode::CONFLICT,
                    "not_ready",
                    "the recovery file exists once setup completes",
                )
            })?;
        Ok(BASE64.encode(sealed))
    }

    /// `POST /api/swaps/{id}/recovery-saved`: the payment request shows only after this.
    pub fn recovery_saved(&mut self, hex_id: &str) -> Result<(), Refusal> {
        self.recovery_file(hex_id)?;
        self.recovery_saved.insert(self.row(hex_id)?.id);
        Ok(())
    }

    /// `POST /api/restore`: takes over a swap from its recovery file (§6.4) and lets the chain
    /// decide where it stands.
    pub fn restore(&mut self, req: &RestoreRequest) -> Result<CreateSwapResponse, Refusal> {
        let pass = self.passphrase.clone().ok_or_else(locked)?;
        let bad = |e: &dyn std::fmt::Display| {
            refuse(StatusCode::BAD_REQUEST, "bad_recovery_file", e.to_string())
        };
        let file = BASE64
            .decode(req.recovery_file.trim())
            .map_err(|e| bad(&e))?;
        let bundle =
            open(&file, SecretString::from(req.passphrase.clone())).map_err(|e| bad(&e))?;
        let quote: Quote = serde_json::from_str(&bundle.quote_json).map_err(|e| bad(&e))?;
        if bundle.role != Role::Alice || quote.network != self.config.network {
            return Err(bad(&"not a taker file for this network"));
        }
        let id = bundle.swap_id;
        if self.store.state(&id).map_err(internal)?.is_some() {
            return Err(refuse(
                StatusCode::CONFLICT,
                "known_swap",
                "this swap is already here",
            ));
        }
        // The file exists only once setup completed (ADR-040); the first tick finds the rest.
        let state = serde_json::to_string(&AliceState::SigsExchanged).map_err(internal)?;
        let network = serde_json::to_string(&quote.network).map_err(internal)?;
        let now = now();
        // Secret first: a swap row without one is aborted on resume.
        self.store
            .put_secret(&id, &seal(&bundle, pass.clone()).map_err(internal)?)
            .map_err(internal)?;
        let new = NewSwap {
            id,
            role: Role::Alice,
            network: &network,
            quote_json: &bundle.quote_json,
            t1_height: u32::from(quote.t1),
            t2_height: u32::from(quote.t2),
            state_json: &state,
        };
        self.store.create_swap(&new, now).map_err(internal)?;
        let row = SwapRow {
            id,
            role: Role::Alice,
            network,
            quote_json: bundle.quote_json.clone(),
            state_json: state,
            t1_height: u32::from(quote.t1),
            t2_height: u32::from(quote.t2),
            created_at: now,
        };
        let runner = AliceRunner::resume(&mut self.store, &row, pass, now)
            .map_err(internal)?
            .ok_or_else(|| internal("restored swap has no session"))?;
        self.restored.insert(id);
        self.recovery_saved.insert(id);
        self.runners.insert(id, runner);
        Ok(CreateSwapResponse {
            swap_id: hex::encode(id),
        })
    }
}
