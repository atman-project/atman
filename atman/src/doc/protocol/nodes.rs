use autosurgeon::{Hydrate, Reconcile};
use serde::{Deserialize, Serialize};

use crate::doc::protocol::node::Node;

pub const DOC_ID: &str = "nodes";

pub const INITIAL_DOC: [u8; 121] = [
    133, 111, 74, 131, 190, 13, 52, 43, 0, 111, 1, 16, 103, 0, 135, 60, 1, 128, 64, 15, 178, 249,
    215, 67, 119, 126, 17, 142, 1, 237, 113, 162, 228, 201, 45, 4, 234, 184, 110, 34, 210, 221, 13,
    50, 180, 235, 107, 15, 79, 88, 147, 219, 144, 119, 83, 195, 254, 44, 104, 72, 193, 6, 1, 2, 3,
    2, 19, 2, 35, 2, 64, 2, 86, 2, 7, 21, 7, 33, 2, 35, 2, 52, 1, 66, 2, 86, 2, 128, 1, 2, 127, 0,
    127, 1, 127, 1, 127, 0, 127, 0, 127, 7, 127, 5, 110, 111, 100, 101, 115, 127, 0, 127, 1, 1,
    127, 2, 127, 0, 127, 0, 0,
];

#[derive(Debug, Clone, Serialize, Deserialize, Reconcile, Hydrate, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Nodes {
    pub nodes: Vec<Node>,
}

impl Nodes {
    pub fn nodes(&self) -> impl Iterator<Item = &Node> {
        self.nodes.iter()
    }
}
