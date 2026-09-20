//! Resources and their reservable capacity units (ADR 0005).

use serde::{Deserialize, Serialize};

use super::ids::{CapacityUnitId, ResourceId};
use super::parse::ParseEnumError;

/// The `capacity_units.kind` discriminant, parsed.
///
/// [`CapacityUnitKind`] carries a pool's capacity on top of this; the
/// claim path only needs to know which of the two algebras applies, so it
/// carries this instead of the bare `"seat"`/`"pool"` string it used to.
/// A string meant every claim site matched with a `_` arm that silently
/// treated anything unrecognised as a pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitKind {
    /// Identity-based: a specific physical place, sold at most once per
    /// segment. Occupancy is a segment bitmask.
    Seat,
    /// Count-based: cargo kilograms, deck slots, standing room. Occupancy
    /// is a per-segment remaining counter.
    Pool,
}

impl UnitKind {
    /// The stored form, matching the `capacity_units.kind` CHECK.
    pub fn as_str(self) -> &'static str {
        match self {
            UnitKind::Seat => "seat",
            UnitKind::Pool => "pool",
        }
    }
}

impl std::fmt::Display for UnitKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for UnitKind {
    type Err = ParseEnumError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "seat" => Ok(UnitKind::Seat),
            "pool" => Ok(UnitKind::Pool),
            other => Err(ParseEnumError::new("UnitKind", other)),
        }
    }
}

/// A vehicle, vessel, or aircraft with a fixed layout of capacity units.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resource {
    pub id: ResourceId,
    pub code: String,
    pub name: String,
    pub kind: ResourceKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Bus,
    Ferry,
    Aircraft,
    Other,
}

/// The generic reservable thing.
///
/// - `Seat`: identity-based — a specific physical place; sold at most once
///   per segment. Occupancy is a segment bitmask.
/// - `Pool`: count-based — cargo kilograms, vehicle deck slots, meals,
///   standing room. Occupancy is a per-segment remaining counter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapacityUnit {
    pub id: CapacityUnitId,
    pub resource_id: ResourceId,
    /// Seat code (`12A`) or pool code (`VEHICLE_DECK`, `CARGO_KG`).
    pub code: String,
    pub kind: CapacityUnitKind,
    /// Required for seats; optional for pools.
    pub fare_class: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapacityUnitKind {
    Seat,
    /// Pool with the given per-segment capacity.
    Pool {
        capacity: i32,
    },
}
