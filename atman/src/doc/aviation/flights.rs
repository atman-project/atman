use autosurgeon::{Hydrate, Reconcile};
use serde::{Deserialize, Serialize};

use super::flight::Flight;

pub const DOC_ID: &str = "flights";

#[derive(Debug, Clone, Serialize, Deserialize, Reconcile, Hydrate, PartialEq, Eq)]
pub struct Flights {
    pub flights: Vec<Flight>,
}

impl Flights {
    pub fn flights(&self) -> impl Iterator<Item = &Flight> {
        self.flights.iter()
    }
}
