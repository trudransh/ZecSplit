//! The maker's HTTP routes (INTERFACES §2): JSON envelopes in, JSON envelopes or 204 out.

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde_json::json;
use tokio::sync::Mutex;
use zecsplit_protocol::messages::{Envelope, Message, decode, encode};

use crate::service::Service;

type Shared = Arc<Mutex<Service>>;

fn reply(env: Option<Envelope>) -> Response {
    match env.map(|e| encode(&e)) {
        None => StatusCode::NO_CONTENT.into_response(),
        Some(Ok(bytes)) => ([(header::CONTENT_TYPE, "application/json")], bytes).into_response(),
        Some(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

fn bad(why: String) -> Response {
    (StatusCode::BAD_REQUEST, why).into_response()
}

fn swap_id(hex_id: &str) -> Result<[u8; 32], String> {
    let mut id = [0; 32];
    hex::decode_to_slice(hex_id, &mut id).map_err(|e| format!("swap id: {e}"))?;
    Ok(id)
}

/// `POST /v1/quote`: a `QuoteRequest` envelope in, a signed `Quote` or an `Error` out.
async fn quote(State(svc): State<Shared>, body: Bytes) -> Response {
    let env = match decode(&body) {
        Ok(env) => env,
        Err(e) => return bad(e.to_string()),
    };
    let Message::QuoteRequest(req) = env.message else {
        return bad("expected QuoteRequest".into());
    };
    reply(Some(svc.lock().await.quote(&req).await))
}

/// `POST /v1/swaps/{id}/msg`: one taker message; the reply, or 204.
async fn message(State(svc): State<Shared>, Path(id): Path<String>, body: Bytes) -> Response {
    let id = match swap_id(&id) {
        Ok(id) => id,
        Err(e) => return bad(e),
    };
    let env = match decode(&body) {
        Ok(env) => env,
        Err(e) => return bad(e.to_string()),
    };
    if env.swap_id != id {
        return bad("swap id in the path and the envelope differ".into());
    }
    reply(svc.lock().await.message(env).await)
}

/// `GET /v1/swaps/{id}/next`: the maker's next message, or 204.
async fn next(State(svc): State<Shared>, Path(id): Path<String>) -> Response {
    match swap_id(&id) {
        Ok(id) => reply(svc.lock().await.next(&id)),
        Err(e) => bad(e),
    }
}

/// `GET /v1/info`: the identity key takers pin, and the quotable range.
async fn info(State(svc): State<Shared>) -> Response {
    let svc = svc.lock().await;
    let (min_zec, max_zec) = svc.limits();
    match svc.identity_key() {
        Ok(key) => axum::Json(json!({
            "v": 1,
            "maker_key": hex::encode(key),
            "min_zec": min_zec,
            "max_zec": max_zec,
        }))
        .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

/// Every INTERFACES §2 route over one service.
pub fn router(svc: Shared) -> Router {
    Router::new()
        .route("/v1/quote", post(quote))
        .route("/v1/swaps/{id}/msg", post(message))
        .route("/v1/swaps/{id}/next", get(next))
        .route("/v1/info", get(info))
        .with_state(svc)
}
