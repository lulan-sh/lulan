//! Passenger types — a fare input (discounts are legally mandated for
//! seniors/PWDs in some markets), matching the passengers table CHECK.

use serde::{Deserialize, Serialize};

use super::parse::ParseEnumError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PassengerType {
    Adult,
    Child,
    Senior,
    Pwd,
    Infant,
}

impl PassengerType {
    /// The stored form, matching the `passengers.passenger_type` CHECK.
    pub fn as_str(self) -> &'static str {
        match self {
            PassengerType::Adult => "adult",
            PassengerType::Child => "child",
            PassengerType::Senior => "senior",
            PassengerType::Pwd => "pwd",
            PassengerType::Infant => "infant",
        }
    }
}

impl std::fmt::Display for PassengerType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for PassengerType {
    type Err = ParseEnumError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "adult" => PassengerType::Adult,
            "child" => PassengerType::Child,
            "senior" => PassengerType::Senior,
            "pwd" => PassengerType::Pwd,
            "infant" => PassengerType::Infant,
            other => return Err(ParseEnumError::new("PassengerType", other)),
        })
    }
}
