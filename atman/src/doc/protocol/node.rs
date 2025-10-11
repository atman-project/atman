use autosurgeon::{Hydrate, Reconcile};
use serde::{Deserialize, Serialize};

pub const DOC_ID: &str = "node";

#[derive(Debug, Clone, Serialize, Deserialize, Reconcile, Hydrate, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    pub id: String,
    pub signature: String,
}
