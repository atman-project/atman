use std::io;

use iroh::endpoint::{
    BindError, ClosedStream, ConnectError, ConnectionError, ReadExactError, WriteError,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Endpoint initialization timed out")]
    EndpointInitTimeout,
    #[error("Network error: {0}")]
    Network(Box<dyn std::error::Error + Send + Sync>),
    #[cfg(feature = "sync")]
    #[error("Sync error: {0}")]
    Sync(#[from] crate::actors::network::protocols::sync::Error),
    #[cfg(feature = "blobs")]
    #[error("Blob error: {0}")]
    Blob(#[from] crate::actors::network::protocols::blobs::Error),
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Network(e.into())
    }
}

impl From<ReadExactError> for Error {
    fn from(e: ReadExactError) -> Self {
        Self::Network(e.into())
    }
}

impl From<WriteError> for Error {
    fn from(e: WriteError) -> Self {
        Self::Network(e.into())
    }
}

impl From<ConnectError> for Error {
    fn from(e: ConnectError) -> Self {
        Self::Network(e.into())
    }
}

impl From<ConnectionError> for Error {
    fn from(e: ConnectionError) -> Self {
        Self::Network(e.into())
    }
}

impl From<BindError> for Error {
    fn from(e: BindError) -> Self {
        Self::Network(e.into())
    }
}

impl From<ClosedStream> for Error {
    fn from(e: ClosedStream) -> Self {
        Self::Network(e.into())
    }
}
