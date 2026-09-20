use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use lulan_engine::inventory::{HoldError, StoreError};
use lulan_engine::ticket::TicketError;
use serde_json::json;

#[derive(Debug)]
pub enum ApiError {
    BadRequest(String),
    Unauthorized(&'static str),
    Forbidden(&'static str),
    NotFound(String),
    Conflict(String),
    TooManyRequests,
    ServiceUnavailable(&'static str),
    Internal(anyhow::Error),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            ApiError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg.to_string()),
            ApiError::Forbidden(msg) => (StatusCode::FORBIDDEN, msg.to_string()),
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            ApiError::Conflict(msg) => (StatusCode::CONFLICT, msg),
            ApiError::TooManyRequests => (
                StatusCode::TOO_MANY_REQUESTS,
                "rate limit exceeded".to_string(),
            ),
            ApiError::ServiceUnavailable(msg) => (StatusCode::SERVICE_UNAVAILABLE, msg.to_string()),
            ApiError::Internal(err) => {
                tracing::error!(error = ?err, "internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal error".to_string(),
                )
            }
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}

impl From<StoreError> for ApiError {
    fn from(err: StoreError) -> Self {
        match err {
            StoreError::UnknownStop(_) | StoreError::StopsOutOfOrder { .. } => {
                ApiError::BadRequest(err.to_string())
            }
            StoreError::Span(_) => ApiError::BadRequest(err.to_string()),
            // The request was well formed; the departure just isn't for
            // sale any more — the same shape as losing a seat race.
            StoreError::TripNotSellable { .. } | StoreError::TripDeparted { .. } => {
                ApiError::Conflict(err.to_string())
            }
            // Both mean the stored data and this build disagree about what
            // is representable. Nothing the caller sends changes that.
            StoreError::UnreplayableStream { .. } | StoreError::UnknownUnitKind(_) => {
                ApiError::Internal(anyhow::anyhow!("{err}"))
            }
            StoreError::Db(db) => ApiError::Internal(db.into()),
        }
    }
}

/// Every database failure that reaches a handler is a 500. Having the
/// conversion here is what lets call sites write `?` instead of repeating
/// `.map_err(|e| ApiError::Internal(e.into()))` at every query.
impl From<sqlx::Error> for ApiError {
    fn from(err: sqlx::Error) -> Self {
        ApiError::Internal(err.into())
    }
}

impl From<TicketError> for ApiError {
    fn from(err: TicketError) -> Self {
        match err {
            TicketError::OrderNotFound => ApiError::NotFound("order not found".into()),
            TicketError::NotPaid(status) => ApiError::Conflict(format!(
                "order is {status}; tickets are issued once payment is captured"
            )),
            // Recoverable by rotating a key, so it is not a 500: the
            // deployment is momentarily unable to sign, not broken.
            TicketError::NoSigningKey => {
                ApiError::ServiceUnavailable("no active ticket signing key")
            }
            TicketError::Store(store) => store.into(),
            other @ (TicketError::Sealing(_) | TicketError::Db(_)) => {
                ApiError::Internal(anyhow::anyhow!("{other}"))
            }
        }
    }
}

/// Losing Redis loses holds, never sold inventory (ADR 0002), so a hold
/// failure is 503 — the same answer as booting without Redis at all —
/// rather than a 500 that implies something is broken.
impl From<HoldError> for ApiError {
    fn from(err: HoldError) -> Self {
        tracing::warn!(error = %err, "hold store unavailable");
        ApiError::ServiceUnavailable("hold service unavailable")
    }
}
