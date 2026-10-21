//! Engine HTTP API types (INTERFACES §3): the web UI's only contract with the engine
//! (ADR-002, ADR-023). `cargo test` exports each type to `packages/engine-api/src/` with
//! ts-rs; the UI imports those files and never hand-writes an API type.
//!
//! Nothing here may carry secret material (AGENTS.md NEVER-2): no shares, keys, `ask`
//! or recovery-file plaintext. The UI renders engine state; it never computes it (T25).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// ZEC in zatoshis (1 ZEC = 10^8). Serialized as a JSON number: total supply fits in 2^53.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Zatoshis(pub u64);

/// BTC in satoshis. Serialized as a JSON number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Sats(pub u64);

/// PROTOCOL_SPEC §2 `network`: `test` is Zcash testnet plus Bitcoin signet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Network {
    Regtest,
    Test,
    MainBeta,
}

/// `GET /api/health`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Health {
    pub version: String,
    pub network: Network,
    /// Whether the secrets DB is unlocked for this session (`POST /api/unlock`).
    pub unlocked: bool,
}

/// `POST /api/unlock`. The passphrase is never stored (ARCHITECTURE §4).
#[derive(Clone, PartialEq, Eq, Deserialize, TS)]
#[ts(export)]
pub struct UnlockRequest {
    pub passphrase: String,
}

/// `POST /api/quote`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct QuoteRequest {
    pub zec_amount: Zatoshis,
}

/// The maker's signed quote, verified by the engine (§2-R2) and rendered for the Quote
/// screen (UX_SPEC §2.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct QuoteView {
    pub quote_id: String,
    pub zec_amount: Zatoshis,
    /// What the user receives at their BTC address.
    pub btc_amount: Sats,
    /// Unix seconds. The UI shows a countdown ("Quote valid for 0:58").
    pub expires_at: u64,
    pub min_zec: Zatoshis,
    pub max_zec: Zatoshis,
    /// Estimated absolute times for the risk panel (UX_SPEC §2.3), from `t1` and `t2`.
    pub refund_window: Window,
}

/// `POST /api/swaps`. The engine refuses unless `accept_risks` is true.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CreateSwapRequest {
    pub quote_id: String,
    pub btc_address: String,
    pub accept_risks: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CreateSwapResponse {
    pub swap_id: String,
}

/// `POST /api/restore`: a recovery file (as exported) and its passphrase (§6.4-R1).
#[derive(Clone, PartialEq, Eq, Deserialize, TS)]
#[ts(export)]
pub struct RestoreRequest {
    pub recovery_file: String,
    pub passphrase: String,
}

/// One row of `GET /api/swaps`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SwapSummary {
    pub swap_id: String,
    pub created_at: u64,
    pub zec_amount: Zatoshis,
    pub btc_amount: Sats,
    pub current_step: StepKind,
    /// `None` while the swap is still running.
    pub outcome: Option<Outcome>,
}

/// `GET /api/swaps/{id}` and every event on `GET /api/swaps/{id}/events` (SSE): the whole
/// view model, so the UI can render any screen from one message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SwapView {
    pub swap_id: String,
    pub network: Network,
    pub zec_amount: Zatoshis,
    pub btc_amount: Sats,
    pub btc_address: String,
    /// The vertical stepper, always in UX_SPEC §2.5 order.
    pub steps: Vec<Step>,
    /// The ZIP 321 URI, built by the engine. Present only once both Bitcoin sources have
    /// confirmed the maker's lock (§4.2-R2) and until the payment is seen.
    pub zip321_uri: Option<String>,
    /// When the user must be back online if the maker misbehaves (USER_FLOWS §3).
    pub refund_window: Window,
    /// Whether the user confirmed saving the recovery file (UX_SPEC §2.4).
    pub recovery_saved: bool,
    pub outcome: Option<Outcome>,
    /// The latest problem, if any. Copy for each code is in UX_SPEC §3.
    pub error: Option<ErrorCode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Step {
    pub kind: StepKind,
    pub status: StepStatus,
    /// Present for on-chain steps: "seen, waiting for confirmations (1 of 3)".
    pub confirmations: Option<Confirmations>,
    /// Full txid. The UI middle-truncates it for display and always copies it in full.
    pub txid: Option<String>,
    pub deadline: Option<Deadline>,
}

/// UX_SPEC §2.5, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum StepKind {
    Setup,
    MakerLocksBtc,
    PayZec,
    MakerVerifies,
    ClaimBtc,
    Done,
}

/// "Never green before it's true" (UX_SPEC §1.3): `Confirmed` means the confirmations
/// the spec requires (§8), never fewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum StepStatus {
    Pending,
    Waiting,
    Seen,
    Confirmed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Confirmations {
    pub have: u32,
    pub need: u32,
}

/// Deadlines are block heights (QUALITY_SYSTEM §2.7); `estimated_at` is only for display,
/// as local time with UTC on hover (UX_SPEC §1.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Deadline {
    pub height: u32,
    pub estimated_at: u64,
}

/// From `t1` (cancel possible) to `t1 + t2` (punish possible): the app must run at some
/// point inside this window if something goes wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Window {
    pub opens_at: u64,
    pub closes_at: u64,
}

/// Terminal results (PROTOCOL_SPEC §6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Outcome {
    /// The user received the BTC.
    Redeemed,
    /// The maker refunded; the user's ZEC came back.
    Refunded,
    /// The maker failed to refund in time; the user received the BTC as a penalty.
    Punished,
    /// Ended before any ZEC was requested. Nothing left the user's wallet.
    Aborted,
}

/// UX_SPEC §3. `ENGINE_OFFLINE` is the UI's own state when it can't reach the engine, so
/// it isn't here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[ts(export)]
pub enum ErrorCode {
    MakerNoLock,
    MakerSilentAfterZec,
    MarginTooSmall,
    WrongZecAmount,
    BtcSourcesDisagree,
}

/// Body of every non-2xx response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ApiError {
    pub code: String,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interfaces_3_wire_names_match_ux_spec() {
        assert_eq!(
            serde_json::to_string(&ErrorCode::BtcSourcesDisagree).unwrap(),
            "\"BTC_SOURCES_DISAGREE\""
        );
        assert_eq!(
            serde_json::to_string(&Network::MainBeta).unwrap(),
            "\"main_beta\""
        );
        assert_eq!(
            serde_json::to_string(&StepKind::PayZec).unwrap(),
            "\"pay_zec\""
        );
        assert_eq!(serde_json::to_string(&Zatoshis(5)).unwrap(), "5");
    }

    #[test]
    fn interfaces_3_create_swap_request_parses_the_documented_body() {
        let body = r#"{"quote_id":"q1","btc_address":"bcrt1qxyz","accept_risks":true}"#;
        let req: CreateSwapRequest = serde_json::from_str(body).unwrap();
        assert!(req.accept_risks);
    }
}
