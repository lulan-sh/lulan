//! Webhook deliveries (Phase 6): the integration surface promised in
//! PRD §8/§9.
//!
//! Two-stage design keeps the outbox relay fast and deliveries durable:
//!
//! 1. [`WebhookSink`] (an [`EventSink`]) fans each outbox event out into
//!    `webhook_deliveries` rows — one per matching active endpoint. No
//!    network I/O on the relay path.
//! 2. [`deliver_due`] (run via [`run_delivery_worker`]) POSTs pending
//!    rows with exponential backoff until delivered or attempts run out.
//!    `FOR UPDATE SKIP LOCKED` lets workers scale horizontally.
//!
//! **No database transaction is ever held across a POST.** Each pass runs
//! claim → POST → record, and only the first and last touch the database:
//!
//! - *claim* takes a batch in one short transaction, marks it `in_flight`
//!   with `claimed_at = now()`, and commits.
//! - *POST* happens with nothing open and all deliveries in flight
//!   concurrently — they are independent, so a slow receiver delays only
//!   itself.
//! - *record* writes each outcome as its own statement.
//!
//! The earlier version POSTed inside the selecting transaction, so twenty
//! unresponsive endpoints could pin a pool connection and twenty row locks
//! for twenty timeouts back to back. A worker that dies between claim and
//! record leaves `in_flight` rows, which the next pass reclaims after
//! [`CLAIM_TIMEOUT_SECS`].
//!
//! Deliveries are signed the Stripe way so receivers can authenticate
//! and reject replays:
//!
//! ```text
//! X-Lulan-Signature: t=<unix>,v1=hex(hmac_sha256(secret, "<t>.<body>"))
//! ```

use chrono::Utc;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::events::{EventSink, StoredEvent};

const MAX_ATTEMPTS: i32 = 8;
/// Base backoff; attempt n waits `BASE * 2^n` seconds (2s … ~4.5min).
const BASE_BACKOFF_SECS: i64 = 2;
/// How many deliveries one pass claims.
const BATCH: i64 = 20;
/// Per-delivery HTTP timeout.
const DELIVERY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// How long a claimed delivery may sit `in_flight` before another worker
/// assumes the claimer died and takes it back. Comfortably longer than
/// [`DELIVERY_TIMEOUT`], so a slow-but-alive POST is never double-sent.
const CLAIM_TIMEOUT_SECS: f64 = 120.0;

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, b| {
            // Writing to a String is infallible.
            let _ = write!(out, "{b:02x}");
            out
        })
}

/// Compute the `X-Lulan-Signature` header value for a payload.
pub fn signature(secret: &str, timestamp: i64, body: &str) -> String {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("hmac accepts any key length");
    mac.update(format!("{timestamp}.{body}").as_bytes());
    format!("t={timestamp},v1={}", hex(&mac.finalize().into_bytes()))
}

/// Verify a received `X-Lulan-Signature` header (receiver-side helper —
/// also what tests use).
pub fn verify_signature(secret: &str, header: &str, body: &str) -> bool {
    let mut timestamp = None;
    let mut sig = None;
    for part in header.split(',') {
        match part.split_once('=') {
            Some(("t", v)) => timestamp = v.parse::<i64>().ok(),
            Some(("v1", v)) => sig = Some(v.to_string()),
            _ => {}
        }
    }
    let (Some(timestamp), Some(sig)) = (timestamp, sig) else {
        return false;
    };
    let expected = signature(secret, timestamp, body);
    // Constant-time compare on the full header string.
    let provided = format!("t={timestamp},v1={sig}");
    expected.len() == provided.len()
        && expected
            .bytes()
            .zip(provided.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

/// EventSink that enqueues one durable delivery per matching endpoint.
#[derive(Debug, Clone)]
pub struct WebhookSink {
    pool: PgPool,
}

impl WebhookSink {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl EventSink for WebhookSink {
    async fn deliver(
        &self,
        event: &StoredEvent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Empty event_types means "everything".
        sqlx::query(
            r#"
            INSERT INTO webhook_deliveries (endpoint_id, event_sequence)
            SELECT id, $1 FROM webhook_endpoints
            WHERE active AND (event_types = '{}' OR $2 = ANY(event_types))
            ON CONFLICT (endpoint_id, event_sequence) DO NOTHING
            "#,
        )
        .bind(event.sequence)
        .bind(&event.event_type)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DeliveryStats {
    pub delivered: usize,
    pub retried: usize,
    pub exhausted: usize,
    /// Rows taken back from a worker that stopped mid-batch. They are
    /// pending again, so the next pass picks them up.
    pub reclaimed: u64,
}

/// One claimed delivery, detached from the database so the POST can run
/// with no transaction open.
#[derive(Debug)]
struct Claimed {
    id: i64,
    /// The attempt number this POST represents — already persisted by the
    /// claim, so a crash mid-flight still counts against the budget.
    attempt: i32,
    url: String,
    secret: String,
    body: String,
}

/// What one POST resolved to.
#[derive(Debug)]
enum Outcome {
    Delivered,
    Failed(String),
}

/// POST one batch of due deliveries. Returns what happened; callers loop.
///
/// Claim, POST, record — see the module docs. Nothing here holds a
/// transaction across the network.
pub async fn deliver_due(
    pool: &PgPool,
    client: &reqwest::Client,
) -> Result<DeliveryStats, sqlx::Error> {
    let mut stats = DeliveryStats {
        reclaimed: reclaim_stale(pool).await?,
        ..DeliveryStats::default()
    };
    if stats.reclaimed > 0 {
        tracing::warn!(
            reclaimed = stats.reclaimed,
            "took back webhook deliveries from a worker that stopped mid-batch"
        );
    }

    let claimed = claim_batch(pool).await?;
    if claimed.is_empty() {
        return Ok(stats);
    }

    // The only phase that touches the network, and it runs with nothing
    // open. Deliveries are independent, so they go concurrently rather
    // than making every receiver wait behind the slowest one.
    let mut posts = tokio::task::JoinSet::new();
    for delivery in claimed {
        let client = client.clone();
        posts.spawn(async move {
            let outcome = post_one(&client, &delivery).await;
            (delivery, outcome)
        });
    }

    while let Some(joined) = posts.join_next().await {
        match joined {
            Ok((delivery, outcome)) => record(pool, &delivery, outcome, &mut stats).await?,
            // The row stays in_flight; reclaim_stale returns it to pending.
            Err(err) => tracing::error!(error = %err, "webhook delivery task failed"),
        }
    }
    Ok(stats)
}

/// Take a batch, mark it in-flight, commit. One short transaction.
async fn claim_batch(pool: &PgPool) -> Result<Vec<Claimed>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let rows = sqlx::query(
        r#"
        SELECT d.id, d.attempts, w.url, w.secret,
               e.sequence, e.stream_id, e.stream_seq, e.event_type,
               e.payload, e.occurred_at
        FROM webhook_deliveries d
        JOIN webhook_endpoints w ON w.id = d.endpoint_id
        JOIN events e ON e.sequence = d.event_sequence
        WHERE d.status = 'pending' AND d.next_attempt_at <= now() AND w.active
        ORDER BY d.next_attempt_at
        LIMIT $1
        FOR UPDATE OF d SKIP LOCKED
        "#,
    )
    .bind(BATCH)
    .fetch_all(&mut *tx)
    .await?;

    if rows.is_empty() {
        return Ok(Vec::new());
    }

    let mut claimed = Vec::with_capacity(rows.len());
    for row in &rows {
        claimed.push(Claimed {
            id: row.try_get("id")?,
            attempt: row.try_get::<i32, _>("attempts")? + 1,
            url: row.try_get("url")?,
            secret: row.try_get("secret")?,
            body: serde_json::json!({
                "sequence": row.try_get::<i64, _>("sequence")?,
                "stream_id": row.try_get::<Uuid, _>("stream_id")?,
                "stream_seq": row.try_get::<i32, _>("stream_seq")?,
                "event_type": row.try_get::<String, _>("event_type")?,
                "payload": row.try_get::<serde_json::Value, _>("payload")?,
                "occurred_at": row.try_get::<chrono::DateTime<Utc>, _>("occurred_at")?,
            })
            .to_string(),
        });
    }

    let ids: Vec<i64> = claimed.iter().map(|c| c.id).collect();
    sqlx::query(
        "UPDATE webhook_deliveries
         SET status = 'in_flight', attempts = attempts + 1, claimed_at = now()
         WHERE id = ANY($1)",
    )
    .bind(&ids)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(claimed)
}

async fn post_one(client: &reqwest::Client, delivery: &Claimed) -> Outcome {
    let timestamp = Utc::now().timestamp();
    let result = client
        .post(&delivery.url)
        .header("content-type", "application/json")
        .header(
            "x-lulan-signature",
            signature(&delivery.secret, timestamp, &delivery.body),
        )
        .body(delivery.body.clone())
        .timeout(DELIVERY_TIMEOUT)
        .send()
        .await;

    match result {
        Ok(response) if response.status().is_success() => Outcome::Delivered,
        Ok(response) => Outcome::Failed(format!("HTTP {}", response.status())),
        Err(err) => Outcome::Failed(err.to_string()),
    }
}

/// Write one outcome. `attempts` was already incremented by the claim.
async fn record(
    pool: &PgPool,
    delivery: &Claimed,
    outcome: Outcome,
    stats: &mut DeliveryStats,
) -> Result<(), sqlx::Error> {
    match outcome {
        Outcome::Delivered => {
            sqlx::query(
                "UPDATE webhook_deliveries
                 SET status = 'delivered', delivered_at = now(), claimed_at = NULL
                 WHERE id = $1",
            )
            .bind(delivery.id)
            .execute(pool)
            .await?;
            stats.delivered += 1;
        }
        Outcome::Failed(error) if delivery.attempt >= MAX_ATTEMPTS => {
            sqlx::query(
                "UPDATE webhook_deliveries
                 SET status = 'failed', last_error = $2, claimed_at = NULL
                 WHERE id = $1",
            )
            .bind(delivery.id)
            .bind(&error)
            .execute(pool)
            .await?;
            tracing::error!(
                delivery_id = delivery.id,
                url = %delivery.url,
                %error,
                "webhook delivery exhausted"
            );
            stats.exhausted += 1;
        }
        Outcome::Failed(error) => {
            let backoff = BASE_BACKOFF_SECS << delivery.attempt.min(8);
            sqlx::query(
                "UPDATE webhook_deliveries
                 SET status = 'pending', last_error = $2, claimed_at = NULL,
                     next_attempt_at = now() + make_interval(secs => $3)
                 WHERE id = $1",
            )
            .bind(delivery.id)
            .bind(&error)
            .bind(backoff as f64)
            .execute(pool)
            .await?;
            stats.retried += 1;
        }
    }
    Ok(())
}

/// Return deliveries whose worker never reported back. Bounded by
/// [`CLAIM_TIMEOUT_SECS`], which is longer than any single POST can take,
/// so this only ever picks up rows whose claimer actually stopped.
async fn reclaim_stale(pool: &PgPool) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query(
        "UPDATE webhook_deliveries
         SET status = 'pending', claimed_at = NULL
         WHERE status = 'in_flight'
           AND claimed_at < now() - make_interval(secs => $1)",
    )
    .bind(CLAIM_TIMEOUT_SECS)
    .execute(pool)
    .await?
    .rows_affected())
}

/// Run the delivery worker forever. Spawn as a background task.
pub async fn run_delivery_worker(pool: PgPool, interval: std::time::Duration) {
    let client = reqwest::Client::new();
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        match deliver_due(&pool, &client).await {
            Ok(stats) if stats.delivered + stats.retried + stats.exhausted > 0 => {
                tracing::info!(
                    delivered = stats.delivered,
                    retried = stats.retried,
                    exhausted = stats.exhausted,
                    reclaimed = stats.reclaimed,
                    "webhook delivery pass"
                );
            }
            Ok(_) => {}
            Err(err) => tracing::error!(error = %err, "webhook delivery pass failed"),
        }
    }
}
