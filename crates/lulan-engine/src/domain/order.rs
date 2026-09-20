//! Order lifecycle state machine. Pure: the single `apply` function is the
//! only way state advances, both for live transitions and event replay —
//! so the read model and the event stream can never disagree about what a
//! sequence of events means.

use serde::{Deserialize, Serialize};

use super::parse::ParseEnumError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderStatus {
    Draft,
    Locked,
    PendingPayment,
    Paid,
    Ticketed,
    Boarded,
    Completed,
    Cancelled,
    Expired,
    Refunded,
}

impl OrderStatus {
    /// The stored form, matching the `orders.status` CHECK.
    pub fn as_str(self) -> &'static str {
        match self {
            OrderStatus::Draft => "draft",
            OrderStatus::Locked => "locked",
            OrderStatus::PendingPayment => "pending_payment",
            OrderStatus::Paid => "paid",
            OrderStatus::Ticketed => "ticketed",
            OrderStatus::Boarded => "boarded",
            OrderStatus::Completed => "completed",
            OrderStatus::Cancelled => "cancelled",
            OrderStatus::Expired => "expired",
            OrderStatus::Refunded => "refunded",
        }
    }

    /// Inventory claims are released when an order leaves these states
    /// without reaching Paid.
    pub fn holds_inventory_provisionally(self) -> bool {
        matches!(self, OrderStatus::Locked | OrderStatus::PendingPayment)
    }
}

impl std::fmt::Display for OrderStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for OrderStatus {
    type Err = ParseEnumError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "draft" => OrderStatus::Draft,
            "locked" => OrderStatus::Locked,
            "pending_payment" => OrderStatus::PendingPayment,
            "paid" => OrderStatus::Paid,
            "ticketed" => OrderStatus::Ticketed,
            "boarded" => OrderStatus::Boarded,
            "completed" => OrderStatus::Completed,
            "cancelled" => OrderStatus::Cancelled,
            "expired" => OrderStatus::Expired,
            "refunded" => OrderStatus::Refunded,
            other => return Err(ParseEnumError::new("OrderStatus", other)),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderEventType {
    OrderCreated,
    InventoryLocked,
    PaymentRequested,
    PaymentCaptured,
    PaymentFailed,
    OrderCancelled,
    OrderExpired,
    TicketIssued,
    PassengerBoarded,
    TripCompleted,
    OrderRefunded,
}

impl OrderEventType {
    /// The stored form, matching the `events.event_type` values.
    pub fn as_str(self) -> &'static str {
        match self {
            OrderEventType::OrderCreated => "order_created",
            OrderEventType::InventoryLocked => "inventory_locked",
            OrderEventType::PaymentRequested => "payment_requested",
            OrderEventType::PaymentCaptured => "payment_captured",
            OrderEventType::PaymentFailed => "payment_failed",
            OrderEventType::OrderCancelled => "order_cancelled",
            OrderEventType::OrderExpired => "order_expired",
            OrderEventType::TicketIssued => "ticket_issued",
            OrderEventType::PassengerBoarded => "passenger_boarded",
            OrderEventType::TripCompleted => "trip_completed",
            OrderEventType::OrderRefunded => "order_refunded",
        }
    }
}

impl std::fmt::Display for OrderEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for OrderEventType {
    type Err = ParseEnumError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "order_created" => OrderEventType::OrderCreated,
            "inventory_locked" => OrderEventType::InventoryLocked,
            "payment_requested" => OrderEventType::PaymentRequested,
            "payment_captured" => OrderEventType::PaymentCaptured,
            "payment_failed" => OrderEventType::PaymentFailed,
            "order_cancelled" => OrderEventType::OrderCancelled,
            "order_expired" => OrderEventType::OrderExpired,
            "ticket_issued" => OrderEventType::TicketIssued,
            "passenger_boarded" => OrderEventType::PassengerBoarded,
            "trip_completed" => OrderEventType::TripCompleted,
            "order_refunded" => OrderEventType::OrderRefunded,
            other => return Err(ParseEnumError::new("OrderEventType", other)),
        })
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TransitionError {
    #[error("event {event:?} is illegal in state {state:?}")]
    Illegal {
        state: OrderStatus,
        event: OrderEventType,
    },
    #[error("event {0:?} cannot start a stream (only order_created can)")]
    NotInitial(OrderEventType),
    #[error("order_created cannot be applied to an existing order in state {0:?}")]
    AlreadyCreated(OrderStatus),
}

/// Apply one event to the current state (`None` = stream start). This is
/// the entire lifecycle:
///
/// Draft → Locked → PendingPayment → Paid → Ticketed → Boarded → Completed
/// with PaymentFailed retrying to Locked, Cancelled/Expired exits before
/// Paid, and Refunded after Paid/Ticketed.
pub fn apply(
    state: Option<OrderStatus>,
    event: OrderEventType,
) -> Result<OrderStatus, TransitionError> {
    use OrderEventType as E;
    use OrderStatus as S;

    let Some(state) = state else {
        return match event {
            E::OrderCreated => Ok(S::Draft),
            other => Err(TransitionError::NotInitial(other)),
        };
    };

    match (state, event) {
        (s, E::OrderCreated) => Err(TransitionError::AlreadyCreated(s)),
        (S::Draft, E::InventoryLocked) => Ok(S::Locked),
        (S::Locked, E::PaymentRequested) => Ok(S::PendingPayment),
        (S::PendingPayment, E::PaymentCaptured) => Ok(S::Paid),
        (S::PendingPayment, E::PaymentFailed) => Ok(S::Locked),
        (S::Locked | S::PendingPayment, E::OrderCancelled) => Ok(S::Cancelled),
        (S::Locked | S::PendingPayment, E::OrderExpired) => Ok(S::Expired),
        (S::Paid, E::TicketIssued) => Ok(S::Ticketed),
        (S::Ticketed, E::PassengerBoarded) => Ok(S::Boarded),
        (S::Boarded, E::TripCompleted) => Ok(S::Completed),
        (S::Paid | S::Ticketed, E::OrderRefunded) => Ok(S::Refunded),
        (state, event) => Err(TransitionError::Illegal { state, event }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use OrderEventType as E;
    use OrderStatus as S;

    fn fold(events: &[E]) -> Result<S, TransitionError> {
        events
            .iter()
            .try_fold(None, |state, &e| apply(state, e).map(Some))
            .map(|s| s.unwrap())
    }

    #[test]
    fn happy_path_reaches_completed() {
        let status = fold(&[
            E::OrderCreated,
            E::InventoryLocked,
            E::PaymentRequested,
            E::PaymentCaptured,
            E::TicketIssued,
            E::PassengerBoarded,
            E::TripCompleted,
        ])
        .unwrap();
        assert_eq!(status, S::Completed);
    }

    #[test]
    fn payment_failure_returns_to_locked_and_is_retryable() {
        let status = fold(&[
            E::OrderCreated,
            E::InventoryLocked,
            E::PaymentRequested,
            E::PaymentFailed,
            E::PaymentRequested,
            E::PaymentCaptured,
        ])
        .unwrap();
        assert_eq!(status, S::Paid);
    }

    #[test]
    fn duplicate_capture_is_illegal_not_state_corrupting() {
        // The webhook handler maps this error to an idempotent no-op.
        let err = apply(Some(S::Paid), E::PaymentCaptured).unwrap_err();
        assert_eq!(
            err,
            TransitionError::Illegal {
                state: S::Paid,
                event: E::PaymentCaptured
            }
        );
    }

    #[test]
    fn paid_orders_cannot_be_cancelled_or_expired() {
        assert!(apply(Some(S::Paid), E::OrderCancelled).is_err());
        assert!(apply(Some(S::Paid), E::OrderExpired).is_err());
        assert!(apply(Some(S::Paid), E::OrderRefunded).is_ok());
    }

    #[test]
    fn only_order_created_starts_a_stream_and_only_once() {
        assert_eq!(apply(None, E::OrderCreated), Ok(S::Draft));
        assert!(matches!(
            apply(None, E::PaymentCaptured),
            Err(TransitionError::NotInitial(_))
        ));
        assert!(matches!(
            apply(Some(S::Locked), E::OrderCreated),
            Err(TransitionError::AlreadyCreated(_))
        ));
    }

    #[test]
    fn status_and_event_strings_roundtrip() {
        for s in [
            S::Draft,
            S::Locked,
            S::PendingPayment,
            S::Paid,
            S::Ticketed,
            S::Boarded,
            S::Completed,
            S::Cancelled,
            S::Expired,
            S::Refunded,
        ] {
            assert_eq!(s.as_str().parse(), Ok(s));
            assert_eq!(s.to_string(), s.as_str());
        }
        for e in [
            E::OrderCreated,
            E::InventoryLocked,
            E::PaymentRequested,
            E::PaymentCaptured,
            E::PaymentFailed,
            E::OrderCancelled,
            E::OrderExpired,
            E::TicketIssued,
            E::PassengerBoarded,
            E::TripCompleted,
            E::OrderRefunded,
        ] {
            assert_eq!(e.as_str().parse(), Ok(e));
            assert_eq!(e.to_string(), e.as_str());
        }
    }

    #[test]
    fn unknown_stored_values_are_refused_with_the_value_named() {
        let err = "teleported".parse::<S>().unwrap_err();
        assert_eq!(err.expected, "OrderStatus");
        assert_eq!(err.value, "teleported");
        assert!("teleported".parse::<E>().is_err());
    }
}
