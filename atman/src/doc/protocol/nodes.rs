use autosurgeon::{Hydrate, Reconcile};
use serde::{Deserialize, Serialize};

use crate::doc::protocol::node::Node;

pub const DOC_ID: &str = "nodes";

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
