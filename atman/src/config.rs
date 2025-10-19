use std::{array::TryFromSliceError, time::Duration};

use serde::{Deserialize, Serialize};

#[cfg(feature = "rest")]
use crate::actors::rest;
use crate::actors::{network, sync};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(with = "secret_key_serde")]
    pub identity: ed25519_dalek::SecretKey,
    pub network: network::Config,
    pub sync: sync::Config,
    #[serde(with = "humantime_serde")]
    pub sync_interval: Option<Duration>,
    #[cfg(feature = "rest")]
    pub rest: rest::Config,
}

pub fn secret_key_from_hex(hex_str: &[u8]) -> Result<ed25519_dalek::SecretKey, SecretKeyError> {
    Ok(ed25519_dalek::SecretKey::try_from(
        hex::decode(hex_str)?.as_slice(),
    )?)
}

#[derive(Debug, thiserror::Error)]
pub enum SecretKeyError {
    #[error("Hex decode error: {0}")]
    HexDecodeError(#[from] hex::FromHexError),
    #[error("Invalid secret key: {0}")]
    InvalidKeyError(#[from] TryFromSliceError),
}

pub mod secret_key_serde {
    use serde::{Deserialize as _, Deserializer, Serialize as _, Serializer, de::Error as _};

    use crate::config::secret_key_from_hex;

    pub fn serialize<S>(key: &ed25519_dalek::SecretKey, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let hex_str = hex::encode(key.as_ref());
        hex_str.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<ed25519_dalek::SecretKey, D::Error>
    where
        D: Deserializer<'de>,
    {
        let hex_str = String::deserialize(deserializer)?;
        secret_key_from_hex(hex_str.as_bytes()).map_err(|e| D::Error::custom(format!("{e}")))
    }
}
