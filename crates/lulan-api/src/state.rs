use std::sync::Arc;

use lulan_engine::inventory::{HoldStore, InventoryStore};
use lulan_engine::orders::OrderStore;
use lulan_engine::payments::{FakeProvider, PaymentProvider};
use lulan_pricing::{NativeEngine, PricingEngine};
use rand::RngCore;
use redis::aio::ConnectionManager;
use sqlx::PgPool;

use crate::error::ApiError;

// Debug is hand-written below: the ports are trait objects.
#[derive(Clone)]
pub struct AppState {
    /// Optional so the server can boot infra-less (health endpoints, CI
    /// smoke tests). Endpoints that need the database return 503 without it.
    pub db: Option<PgPool>,
    /// Optional by design, not just for dev: holds degrade to 503 while
    /// claims stay correct (ADR 0002).
    pub redis: Option<ConnectionManager>,
    /// The active pricing engine — native by default, an operator WASM
    /// module when `LULAN_PRICING_WASM` is set (ADR 0003).
    pub pricing: Arc<dyn PricingEngine>,
    /// The active payment provider (ADR-style port): a configured PSP, or
    /// FakeProvider when none is set. Never constructed inline by a
    /// handler — the running adapter is deployment configuration.
    pub payments: Arc<dyn PaymentProvider>,
    /// HMAC key for quote tokens. `Arc<[u8]>` rather than `Arc<Vec<u8>>`:
    /// one allocation and one pointer hop instead of two.
    pub quote_secret: Arc<[u8]>,
    /// Customer identity port (operator's IdP). None = guest checkout
    /// only; customer endpoints return 401.
    pub identity: Option<Arc<dyn crate::identity::IdentityProvider>>,
    /// Per-caller rate limits, resolved at boot so the request path never
    /// reads the environment.
    pub limits: crate::config::RuntimeLimits,
    /// Request timeout applied by the outermost layer.
    pub request_timeout_secs: u64,
}

/// Reports which ports are wired rather than their innards — that is what
/// you want in a boot log or a handler's error context, and the trait
/// objects have nothing printable anyway.
impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("db", &self.db.is_some())
            .field("redis", &self.redis.is_some())
            .field("payments", &self.payments.name())
            .field("identity", &self.identity.is_some())
            .field("limits", &self.limits)
            .field("request_timeout_secs", &self.request_timeout_secs)
            .finish_non_exhaustive()
    }
}

impl AppState {
    /// Native pricing engine and an ephemeral quote secret — what tests
    /// and infra-less boots want. Loads/creates the ticket signing key
    /// when a database is present.
    pub async fn new(db: Option<PgPool>, redis: Option<ConnectionManager>) -> Self {
        // Ensure a signing key exists; issuance reads the ACTIVE key at
        // sign time so rotation lands without a restart.
        if let Some(pool) = &db
            && let Err(err) = lulan_engine::ticket::TicketSigner::load_or_create(pool).await
        {
            tracing::error!(error = %err, "ticket signing key unavailable");
        }
        let identity = crate::identity::provider_from_env().await;
        Self {
            db,
            redis,
            pricing: Arc::new(NativeEngine),
            payments: Arc::new(FakeProvider),
            quote_secret: ephemeral_secret(),
            identity,
            limits: crate::config::RuntimeLimits::default(),
            request_timeout_secs: 30,
        }
    }

    pub fn inventory(&self) -> Result<InventoryStore, ApiError> {
        self.db
            .clone()
            .map(InventoryStore::new)
            .ok_or(ApiError::ServiceUnavailable("database not configured"))
    }

    pub fn orders(&self) -> Result<OrderStore, ApiError> {
        self.db
            .clone()
            .map(OrderStore::new)
            .ok_or(ApiError::ServiceUnavailable("database not configured"))
    }

    pub fn holds(&self) -> Result<HoldStore, ApiError> {
        self.redis
            .clone()
            .map(HoldStore::new)
            .ok_or(ApiError::ServiceUnavailable(
                "hold service unavailable (no Redis)",
            ))
    }
}

/// 32 random bytes from the OS CSPRNG. Quotes signed with an ephemeral
/// secret die on restart, which is fine — clients just re-quote.
///
/// Drawn from `rand` rather than concatenated UUIDs: two v4 UUIDs carry
/// 244 bits, not 256, because twelve of their bits are fixed version and
/// variant markers — and relying on a UUID generator as a source of key
/// material makes a cryptographic assumption out of an implementation
/// detail of that crate.
pub fn ephemeral_secret() -> Arc<[u8]> {
    let mut secret = [0u8; 32];
    rand::rng().fill_bytes(&mut secret);
    Arc::from(secret.as_slice())
}
