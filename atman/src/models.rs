use std::str::FromStr as _;

use iroh::{EndpointId, KeyParsingError};

use crate::doc;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Node {
    pub id: EndpointId,
}

impl TryFrom<&doc::protocol::node::Node> for Node {
    type Error = KeyParsingError;

    fn try_from(node: &doc::protocol::node::Node) -> Result<Self, Self::Error> {
        Ok(Self {
            id: EndpointId::from_str(node.id.as_str())?,
        })
    }
}

impl From<&Node> for doc::protocol::node::Node {
    fn from(node: &Node) -> Self {
        Self {
            id: node.id.to_string(),
            signature: String::new(),
        }
    }
}
