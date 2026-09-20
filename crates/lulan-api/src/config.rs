/// Server configuration, read from environment variables.
#[derive(Debug, Clone)]
pub struct Config {
    /// Listen address, e.g. `0.0.0.0:8080`. Env: `LULAN_LISTEN_ADDR`.
    pub listen_addr: String,
    /// Postgres connection string. Env: `DATABASE_URL`. Optional in early
    /// phases so the server can boot without infrastructure.
    pub database_url: Option<String>,
    /// Redis connection string for soft holds. Env: `REDIS_URL`. Optional:
    /// without it, hold endpoints return 503 but claims still work — Redis
    /// is never required for correctness (ADR 0002).
    pub redis_url: Option<String>,
    /// Path to an operator pricing module (`.wasm`). Env:
    /// `LULAN_PRICING_WASM`. Unset = native rule engine.
    pub pricing_wasm: Option<String>,
    /// HMAC key for quote tokens. Env: `LULAN_QUOTE_SECRET`. Unset = a
    /// random per-boot secret (quotes won't survive restarts).
    pub quote_secret: Option<String>,
    /// Payment provider: a built-in preset name (`stripe`) or a path to a
    /// provider description. Env: `LULAN_PAYMENT_PROVIDER`. Unset = the
    /// fake provider, which takes no money.
    pub payment_provider: Option<String>,
    /// Postgres pool size. Env: `LULAN_DB_POOL`.
    pub db_pool: u32,
    /// How long a request may run before 408. Env:
    /// `LULAN_REQUEST_TIMEOUT_SECS`; 0 or unparseable falls back.
    pub request_timeout_secs: u64,
    /// Per-caller rate limits, read once (see [`RuntimeLimits`]).
    pub limits: RuntimeLimits,
}

/// Settings the request path consults on every call.
///
/// Read once at boot and carried, rather than re-read from the
/// environment per request: `std::env::var` takes a process-wide lock and
/// allocates, and doing it twice per request put that in the hot path of
/// the rate limiter. Reading once also means a misspelled variable is a
/// wrong value from the first request rather than a silent default
/// forever.
#[derive(Debug, Clone, Copy)]
pub struct RuntimeLimits {
    /// `LULAN_RATE_LIMIT` — writes per minute per caller.
    pub write_per_minute: u64,
    /// `LULAN_RATE_LIMIT_READS` — reads per minute per caller.
    pub read_per_minute: u64,
    /// `LULAN_TRUSTED_PROXY_HOPS` — how many reverse proxies sit in front.
    /// Zero means `X-Forwarded-For` is ignored entirely.
    pub trusted_proxy_hops: usize,
}

pub(crate) const DEFAULT_WRITE_LIMIT: u64 = 300;
pub(crate) const DEFAULT_READ_LIMIT: u64 = 1_200;
const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 30;
const DEFAULT_DB_POOL: u32 = 32;

/// Parse an env var, falling back when unset or unparseable.
fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

impl Default for RuntimeLimits {
    fn default() -> Self {
        Self {
            write_per_minute: DEFAULT_WRITE_LIMIT,
            read_per_minute: DEFAULT_READ_LIMIT,
            trusted_proxy_hops: 0,
        }
    }
}

impl RuntimeLimits {
    pub fn from_env() -> Self {
        Self {
            write_per_minute: env_or("LULAN_RATE_LIMIT", DEFAULT_WRITE_LIMIT),
            read_per_minute: env_or("LULAN_RATE_LIMIT_READS", DEFAULT_READ_LIMIT),
            trusted_proxy_hops: env_or("LULAN_TRUSTED_PROXY_HOPS", 0),
        }
    }
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            listen_addr: std::env::var("LULAN_LISTEN_ADDR")
                .unwrap_or_else(|_| "0.0.0.0:8080".to_string()),
            database_url: std::env::var("DATABASE_URL").ok(),
            redis_url: std::env::var("REDIS_URL").ok(),
            pricing_wasm: std::env::var("LULAN_PRICING_WASM").ok(),
            quote_secret: std::env::var("LULAN_QUOTE_SECRET").ok(),
            payment_provider: std::env::var("LULAN_PAYMENT_PROVIDER").ok(),
            db_pool: env_or("LULAN_DB_POOL", DEFAULT_DB_POOL),
            request_timeout_secs: match env_or(
                "LULAN_REQUEST_TIMEOUT_SECS",
                DEFAULT_REQUEST_TIMEOUT_SECS,
            ) {
                0 => DEFAULT_REQUEST_TIMEOUT_SECS,
                secs => secs,
            },
            limits: RuntimeLimits::from_env(),
        }
    }
}
