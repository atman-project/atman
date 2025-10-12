use autosurgeon::{Hydrate, Reconcile};
use serde::{Deserialize, Serialize};

use super::flight::Flight;

pub const DOC_ID: &str = "flights";

pub const INITIAL_DOC: [u8; 123] = [
    133, 111, 74, 131, 6, 162, 182, 148, 0, 113, 1, 16, 213, 179, 153, 225, 238, 86, 65, 226, 132,
    140, 91, 46, 208, 192, 42, 128, 1, 70, 59, 210, 81, 164, 26, 169, 212, 88, 70, 131, 79, 97,
    130, 178, 109, 141, 124, 34, 184, 141, 203, 78, 226, 126, 224, 63, 160, 235, 235, 113, 184, 6,
    1, 2, 3, 2, 19, 2, 35, 2, 64, 2, 86, 2, 7, 21, 9, 33, 2, 35, 2, 52, 1, 66, 2, 86, 2, 128, 1, 2,
    127, 0, 127, 1, 127, 1, 127, 0, 127, 0, 127, 7, 127, 7, 102, 108, 105, 103, 104, 116, 115, 127,
    0, 127, 1, 1, 127, 2, 127, 0, 127, 0, 0,
];

#[derive(Debug, Clone, Serialize, Deserialize, Reconcile, Hydrate, PartialEq, Eq, Default)]
pub struct Flights {
    pub flights: Vec<Flight>,
}

impl Flights {
    pub fn flights(&self) -> impl Iterator<Item = &Flight> {
        self.flights.iter()
    }
}
