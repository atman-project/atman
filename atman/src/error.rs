use crate::{actors, doc};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Network error: {0}")]
    Network(#[from] actors::network::Error),
    #[error("Sync error: {0}")]
    Sync(#[from] actors::sync::Error),
    #[cfg(feature = "rest")]
    #[error("Sync error: {0}")]
    Http(#[from] actors::rest::Error),
    #[error("Document error: {0}")]
    Doc(#[from] doc::Error),
    #[error("Invalid config: {0}")]
    InvalidConfig(String),
    #[error("Unexpected document type")]
    UnexpectedDocumentType,
}
