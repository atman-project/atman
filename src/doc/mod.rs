pub mod aviation;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocSpace(String);

impl From<String> for DocSpace {
    fn from(space: String) -> Self {
        DocSpace(space)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocId(String);

impl From<String> for DocId {
    fn from(id: String) -> Self {
        DocId(id)
    }
}
