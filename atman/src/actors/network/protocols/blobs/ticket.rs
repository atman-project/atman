//! A wrapper around [`iroh_blobs::BlobTicket`].
//!
//! [`iroh_blobs::BlobTicket`] already encodes the sender's endpoint ID, relay
//! url and direct addresses, plus the blob hash and format — everything a
//! receiver needs to dial back and pull the bytes. The one thing it doesn't
//! carry is the original filename. Without that, the receiver has no good way
//! to name the file when it lands on disk.
//!
//! This module wraps a [`iroh_blobs::BlobTicket`] together with the filename.

use std::{fmt, str::FromStr};

use iroh_blobs::ticket::BlobTicket as IrohBlobTicket;
use serde::{Deserialize, Serialize};

const PREFIX: &str = "atman-blob1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobTicket {
    pub inner: IrohBlobTicket,
    pub filenames: Vec<String>,
}

impl BlobTicket {
    pub fn new(inner: IrohBlobTicket, filenames: Vec<String>) -> Self {
        Self { inner, filenames }
    }
}

impl fmt::Display for BlobTicket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let bytes = postcard::to_allocvec(self).map_err(|_| fmt::Error)?;
        let encoded = data_encoding::BASE32_NOPAD.encode(&bytes).to_lowercase();
        write!(f, "{PREFIX}{encoded}")
    }
}

impl FromStr for BlobTicket {
    type Err = BlobTicketError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let body = s
            .strip_prefix(PREFIX)
            .ok_or(BlobTicketError::MissingPrefix)?;
        let bytes = data_encoding::BASE32_NOPAD
            .decode(body.to_ascii_uppercase().as_bytes())
            .map_err(|_| BlobTicketError::BadBase32)?;
        postcard::from_bytes(&bytes).map_err(|_| BlobTicketError::BadPayload)
    }
}

#[derive(thiserror::Error, Debug)]
pub enum BlobTicketError {
    #[error("ticket must start with `{PREFIX}`")]
    MissingPrefix,
    #[error("ticket body is not valid base32-nopad")]
    BadBase32,
    #[error("ticket payload is not a valid atman blob ticket")]
    BadPayload,
}

#[cfg(test)]
mod tests {
    use iroh_blobs::{BlobFormat, Hash};

    use super::*;

    #[test]
    fn roundtrip() {
        let endpoint_id = iroh::SecretKey::from_bytes(&[7u8; 32]).public();
        let blob = IrohBlobTicket::new(
            endpoint_id.into(),
            Hash::from_bytes([1u8; 32]),
            BlobFormat::Raw,
        );
        let t = BlobTicket::new(blob, vec!["hello.txt".to_string()]);
        let s = t.to_string();
        assert!(s.starts_with(PREFIX));
        let parsed: BlobTicket = s.parse().unwrap();
        assert_eq!(parsed.filenames, vec!["hello.txt"]);
        assert_eq!(parsed.inner.hash(), t.inner.hash());
    }
}
