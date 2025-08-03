use autosurgeon::{Hydrate, Reconcile};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, Reconcile, Hydrate)]
#[serde(rename_all = "camelCase")]
pub struct Flight {
    id: Uuid,
    departure_airport: String,
    arrival_airport: String,
    departure_local_time: String,
    arrival_local_time: String,
    airline: String,
    aircraft: String,
    flight_number: String,
    booking_reference: String,
}
