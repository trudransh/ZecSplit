//! The engine's localhost API (INTERFACES §3): an origin allowlist, a session token, JSON and SSE.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;
use tokio::sync::Mutex;

use crate::api::{CreateSwapRequest, QuoteRequest, RestoreRequest, UnlockRequest};
use crate::engine::{Engine, Refusal};

type Shared = Arc<Mutex<Engine>>;

/// Who may call the API: the UI's origins and the token handed to it at start.
pub struct Access {
    pub token: String,
    pub origins: Vec<String>,
}

fn refusal(r: Refusal) -> Response {
    (r.status, Json(r.body)).into_response()
}

fn respond<T: Serialize>(r: Result<T, Refusal>) -> Response {
    match r {
        Ok(v) => Json(v).into_response(),
        Err(e) => refusal(e),
    }
}

/// Compares without an early exit, so the token can't be guessed byte by byte.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0, |acc, (x, y)| acc | (x ^ y))
            == 0
}

/// `Authorization: Bearer <token>`, or `?token=` for `EventSource`, which can't set headers.
fn token(req: &Request) -> Option<&str> {
    let bearer = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    let query = req
        .uri()
        .query()
        .and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("token=")));
    bearer.or(query)
}

/// INTERFACES §3 security: a foreign `Origin` is refused outright, then every call needs the
/// token. Allowed origins get CORS headers and Private Network Access consent, since the UI may be
/// served from a public site while the engine listens on loopback (ADR-055).
async fn guard(State(access): State<Arc<Access>>, req: Request, next: Next) -> Response {
    let origin = req.headers().get(header::ORIGIN).cloned();
    if let Some(o) = &origin
        && !access.origins.iter().any(|a| a.as_bytes() == o.as_bytes())
    {
        return (StatusCode::FORBIDDEN, "origin not allowed").into_response();
    }
    let mut res = if req.method() == Method::OPTIONS {
        StatusCode::NO_CONTENT.into_response()
    } else if token(&req).is_some_and(|t| same(t, &access.token)) {
        next.run(req).await
    } else {
        (StatusCode::UNAUTHORIZED, "missing or wrong token").into_response()
    };
    if let Some(o) = origin {
        let h = res.headers_mut();
        h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, o);
        h.insert(header::VARY, HeaderValue::from_static("Origin"));
        h.insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static("authorization, content-type"),
        );
        h.insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("GET, POST"),
        );
        h.insert(
            "access-control-allow-private-network",
            HeaderValue::from_static("true"),
        );
    }
    res
}

async fn health(State(e): State<Shared>) -> Response {
    Json(e.lock().await.health()).into_response()
}

async fn unlock(State(e): State<Shared>, Json(req): Json<UnlockRequest>) -> Response {
    respond(e.lock().await.unlock(&req.passphrase).await)
}

async fn quote(State(e): State<Shared>, Json(req): Json<QuoteRequest>) -> Response {
    respond(e.lock().await.quote(req.zec_amount.0).await)
}

async fn create_swap(State(e): State<Shared>, Json(req): Json<CreateSwapRequest>) -> Response {
    respond(e.lock().await.create_swap(&req).await)
}

async fn swaps(State(e): State<Shared>) -> Response {
    respond(e.lock().await.swaps())
}

async fn swap(State(e): State<Shared>, Path(id): Path<String>) -> Response {
    respond(e.lock().await.swap(&id))
}

async fn recovery_file(State(e): State<Shared>, Path(id): Path<String>) -> Response {
    match e.lock().await.recovery_file(&id) {
        Ok(text) => (
            [
                (header::CONTENT_TYPE, "text/plain".to_string()),
                (
                    header::CONTENT_DISPOSITION,
                    format!(
                        "attachment; filename=\"zecsplit-{}.recovery\"",
                        &id[..8.min(id.len())]
                    ),
                ),
            ],
            text,
        )
            .into_response(),
        Err(r) => refusal(r),
    }
}

async fn recovery_saved(State(e): State<Shared>, Path(id): Path<String>) -> Response {
    match e.lock().await.recovery_saved(&id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(r) => refusal(r),
    }
}

async fn restore(State(e): State<Shared>, Json(req): Json<RestoreRequest>) -> Response {
    respond(e.lock().await.restore(&req))
}

/// `GET /api/swaps/{id}/events`: the whole view model each time it changes.
async fn events(State(e): State<Shared>, Path(id): Path<String>) -> Response {
    if let Err(r) = e.lock().await.swap(&id) {
        return refusal(r);
    }
    let stream = futures_util::stream::unfold((e, id, String::new()), |(e, id, last)| async move {
        loop {
            let view = e.lock().await.swap(&id).ok()?;
            let json = serde_json::to_string(&view).ok()?;
            if json != last {
                let event = Event::default().data(json.clone());
                return Some((Ok::<_, Infallible>(event), (e, id, json)));
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// Every INTERFACES §3 route over one engine, behind the guard.
pub fn router(engine: Shared, access: Access) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/unlock", post(unlock))
        .route("/api/quote", post(quote))
        .route("/api/swaps", post(create_swap).get(swaps))
        .route("/api/swaps/{id}", get(swap))
        .route("/api/swaps/{id}/recovery-file", get(recovery_file))
        .route("/api/swaps/{id}/recovery-saved", post(recovery_saved))
        .route("/api/swaps/{id}/events", get(events))
        .route("/api/restore", post(restore))
        .with_state(engine)
        .layer(middleware::from_fn_with_state(Arc::new(access), guard))
}
