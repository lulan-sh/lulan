//! The error every domain enum's [`FromStr`] returns.
//!
//! These enums mirror columns with CHECK constraints, so a value that does
//! not parse means the database and this build disagree about the allowed
//! set — a deployment skew, not a user mistake. Naming the type and the
//! value is what makes that diagnosable from a log line.
//!
//! [`FromStr`]: std::str::FromStr

/// A string did not match any variant of `expected`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{value:?} is not a valid {expected}")]
pub struct ParseEnumError {
    /// The type that refused it, for the log line.
    pub expected: &'static str,
    pub value: String,
}

impl ParseEnumError {
    /// Public so crates outside the engine can give their own
    /// CHECK-backed enums the same `FromStr` error.
    pub fn new(expected: &'static str, value: &str) -> Self {
        Self {
            expected,
            value: value.to_string(),
        }
    }
}
