use autosurgeon::{Hydrate, Reconcile};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const DOC_ID: &str = "flight";

#[derive(Debug, Clone, Serialize, Deserialize, Reconcile, Hydrate, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Flight {
    pub id: Uuid,
    pub departure_airport: String,
    pub arrival_airport: String,
    pub departure_local_time: String,
    pub arrival_local_time: String,
    pub airline: String,
    pub aircraft: String,
    pub flight_number: String,
    pub booking_reference: String,
}
