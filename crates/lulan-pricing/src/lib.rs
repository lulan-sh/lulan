//! Lulan pricing (ADR 0003): the stable contract is [`PricingEngine`];
//! runtimes are interchangeable.
//!
//! - [`rules`] — the pure, deterministic evaluation core and wire types.
//!   Always available; compiles to any target including wasm32 (this is
//!   what pricing modules link against, `default-features = false`).
//! - [`NativeEngine`] — evaluates rules in-process (`host` feature).
//! - [`WasmEngine`] — runs an operator-supplied WASM module under
//!   wasmtime, fuel-metered and memory-capped (`host` feature).
//!
//! The module ABI is defined in `wit/pricing.wit`.

// Every public type is inspectable: a request or response you cannot
// put in a `tracing` field is one you cannot diagnose in production.
#![warn(missing_debug_implementations)]

pub mod rules;

#[cfg(feature = "host")]
mod native;
#[cfg(feature = "host")]
mod wasm;

#[cfg(feature = "host")]
pub use native::NativeEngine;
#[cfg(feature = "host")]
pub use wasm::WasmEngine;

use rules::{EvalError, FareRuleSet, Quote, RuleInput};

/// Why a price could not be produced.
///
/// Split by *whose fault it is*, because the HTTP layer has to choose a
/// status code and "the operator's module is broken" is not a 400. A
/// single stringly-typed variant made every failure look like bad input.
#[derive(Debug, thiserror::Error)]
pub enum PricingError {
    /// The rules rejected this input — unknown fare key, bad quantity.
    /// The caller can fix it by asking for something else.
    #[error(transparent)]
    Eval(#[from] EvalError),
    /// An operator-supplied module rejected the input on its own terms.
    /// Caller-fixable, like [`Eval`](PricingError::Eval).
    #[error("pricing module rejected the input: {0}")]
    Rejected(String),
    /// The module is unusable: it failed to compile, does not export the
    /// ABI, or returned something that could not be decoded. Nothing the
    /// caller sends will change this — it is a deployment fault.
    #[error("pricing module is unusable: {0}")]
    Unusable(String),
    /// The module ran and died: trapped, or exhausted its fuel budget.
    /// Also a deployment fault, and the one worth alerting on.
    #[error("pricing module failed at runtime: {0}")]
    Trapped(String),
}

impl PricingError {
    /// True when the caller could plausibly succeed by sending something
    /// different; false when the deployment is at fault.
    pub fn is_caller_fault(&self) -> bool {
        matches!(self, PricingError::Eval(_) | PricingError::Rejected(_))
    }
}

/// One line item in, one quote out. Synchronous by design: pricing is a
/// bounded CPU-only computation (the <5 ms PRD target), never I/O.
///
/// # Examples
///
/// ```
/// use lulan_pricing::rules::{FareRuleSet, RuleInput};
/// use lulan_pricing::{NativeEngine, PricingEngine};
///
/// let rules: FareRuleSet = serde_json::from_str(
///     r#"{"currency": "PHP", "base_fare_per_segment": {"ECONOMY": 50000}}"#,
/// )?;
/// let input = RuleInput {
///     fare_key: "ECONOMY".into(),
///     segments: 1,
///     quantity: 2,
///     ..Default::default()
/// };
///
/// let quote = NativeEngine.price(&rules, &input)?;
/// assert_eq!(quote.currency, "PHP");
/// assert_eq!(quote.total_minor, 100_000); // 2 x 500.00 PHP
///
/// // An unpriced fare key is the caller's problem, not the deployment's.
/// let unknown = RuleInput { fare_key: "SUITE".into(), ..input };
/// assert!(NativeEngine.price(&rules, &unknown).unwrap_err().is_caller_fault());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub trait PricingEngine: Send + Sync {
    fn price(&self, rules: &FareRuleSet, input: &RuleInput) -> Result<Quote, PricingError>;

    /// Whether [`price`] may run long enough that it must not occupy an
    /// async runtime thread.
    ///
    /// [`NativeEngine`] evaluates a fixed rule list in microseconds and
    /// answers `false`: hopping to another thread would cost more than the
    /// work. [`WasmEngine`] answers `true` — an operator module is
    /// untrusted code with a large fuel budget, and a module that loops
    /// until it traps would otherwise stall a runtime worker for the whole
    /// budget, with no way for tokio to preempt it. Hosts check this and
    /// hand those calls to a blocking thread.
    ///
    /// [`price`]: PricingEngine::price
    fn runs_untrusted_code(&self) -> bool {
        false
    }
}
