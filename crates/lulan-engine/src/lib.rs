//! Lulan core engine.
//!
//! Modules (built out across development phases — see `docs/development-plan.md`):
//! - [`domain`] — pure domain types and invariants (Phase 1)
//! - `inventory` — segment availability, holds, claims (Phases 1–2)
//! - `events` — append-only event log and outbox (Phase 3)
//! - `orders` — order lifecycle state machine (Phase 3)
//! - `ticket` — ticket issuance and signing (Phase 5)

// Every public type is inspectable: a request or response you cannot
// put in a `tracing` field is one you cannot diagnose in production.
#![warn(missing_debug_implementations)]

pub mod domain;
pub mod events;
pub mod inventory;
pub mod orders;
pub mod payments;
pub mod ticket;
pub mod webhooks;
