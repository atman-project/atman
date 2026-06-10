//! UniFFI surface — the Swift / Kotlin entry point.

#[cfg(feature = "blobs")]
use std::path::PathBuf;
#[cfg(any(feature = "blobs", feature = "sync"))]
use std::str::FromStr;
use std::sync::Arc;

use once_cell::sync::OnceCell;
use tokio::sync::{Mutex, mpsc, oneshot};
#[cfg(feature = "sync")]
use tracing::info;

use crate::{Atman, Command, Config, actors::network, config::secret_key_from_hex};

#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum AtmanError {
    #[error("invalid identity")]
    InvalidIdentity,
    #[error("invalid network key")]
    InvalidNetworkKey,
    #[error("invalid relay url: {0}")]
    InvalidRelayUrl(String),
    #[error("invalid node id: {0}")]
    InvalidNodeId(String),
    #[error("invalid ticket: {0}")]
    InvalidTicket(String),
    #[error("invalid utf-8: {0}")]
    InvalidUtf8(String),
    #[error("send/receive channel closed")]
    ChannelClosed,
    #[error("internal: {0}")]
    Internal(String),
}

#[derive(uniffi::Object)]
pub struct AtmanClient {
    // `Mutex` because UniFFI's `Arc<Self>` receiver convention rules
    // out `&mut self` on the exported async methods.
    command_sender: Mutex<mpsc::Sender<Command>>,
}

#[uniffi::export]
impl AtmanClient {
    /// Spin up the node and block until it's ready to accept commands.
    ///
    /// `sync_*` params are ignored when the `sync` feature is off; they
    /// stay in the signature so the generated FFI surface is stable
    /// across feature combos.
    #[uniffi::constructor(async_runtime = "tokio")]
    pub async fn new(
        identity_hex: String,
        network_key_hex: String,
        custom_relay_url: Option<String>,
        syncman_dir: String,
        sync_interval_secs: u64,
    ) -> Result<Arc<Self>, AtmanError> {
        #[cfg(not(feature = "sync"))]
        {
            let _ = &syncman_dir;
            let _ = sync_interval_secs;
        }
        init_tracing();

        let identity = secret_key_from_hex(identity_hex.as_bytes())
            .map_err(|_| AtmanError::InvalidIdentity)?;
        let network_key = secret_key_from_hex(network_key_hex.as_bytes())
            .map_err(|_| AtmanError::InvalidNetworkKey)?;

        let custom_relay = match custom_relay_url {
            None => None,
            Some(s) => Some(
                s.parse()
                    .map_err(|e: url::ParseError| AtmanError::InvalidRelayUrl(e.to_string()))?,
            ),
        };

        let config = Config {
            identity,
            network: network::Config {
                key: Some(iroh::SecretKey::from_bytes(&network_key)),
                custom_relay_url: custom_relay,
            },
            #[cfg(feature = "sync")]
            sync: crate::SyncConfig {
                syncman_dir: PathBuf::from(syncman_dir),
            },
            #[cfg(feature = "sync")]
            sync_interval: if sync_interval_secs == 0 {
                None
            } else {
                Some(std::time::Duration::from_secs(sync_interval_secs))
            },
            #[cfg(feature = "rest")]
            rest: Default::default(),
        };

        let (atman, command_sender) =
            Atman::new(config).map_err(|e| AtmanError::Internal(e.to_string()))?;

        let (ready_sender, ready_receiver) = oneshot::channel();
        tokio::spawn(async move { atman.run(ready_sender).await });
        ready_receiver
            .await
            .map_err(|_| AtmanError::ChannelClosed)?
            .map_err(|e| AtmanError::Internal(e.to_string()))?;

        tracing::info!("AtmanClient ready");
        Ok(Arc::new(Self {
            command_sender: Mutex::new(command_sender),
        }))
    }
}

// `#[uniffi::export]` can't handle per-method `#[cfg]` gates, so the
// feature-gated methods live in their own impl blocks.

#[cfg(feature = "blobs")]
#[uniffi::export(async_runtime = "tokio")]
impl AtmanClient {
    /// Import files into the blob store; returns a shareable ticket.
    pub async fn send_files(&self, paths: Vec<String>) -> Result<String, AtmanError> {
        let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
        let (reply_sender, reply_receiver) = oneshot::channel();
        self.command_sender
            .lock()
            .await
            .send(Command::Blobs(crate::command::blobs::Command::SendFiles {
                paths,
                reply_sender,
            }))
            .await
            .map_err(|_| AtmanError::ChannelClosed)?;
        let ticket = reply_receiver
            .await
            .map_err(|_| AtmanError::ChannelClosed)?
            .map_err(|e| AtmanError::Internal(e.to_string()))?;
        Ok(ticket.to_string())
    }

    /// Pull every blob in `ticket` into `save_dir`; returns saved paths.
    pub async fn download_files(
        &self,
        ticket: String,
        save_dir: String,
    ) -> Result<Vec<String>, AtmanError> {
        let parsed = crate::BlobTicket::from_str(&ticket)
            .map_err(|e| AtmanError::InvalidTicket(e.to_string()))?;
        let (reply_sender, reply_receiver) = oneshot::channel();
        self.command_sender
            .lock()
            .await
            .send(Command::Blobs(
                crate::command::blobs::Command::DownloadFiles {
                    ticket: parsed,
                    save_dir: PathBuf::from(save_dir),
                    reply_sender,
                },
            ))
            .await
            .map_err(|_| AtmanError::ChannelClosed)?;
        let paths = reply_receiver
            .await
            .map_err(|_| AtmanError::ChannelClosed)?
            .map_err(|e| AtmanError::Internal(e.to_string()))?;
        Ok(paths
            .into_iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect())
    }

    /// Distinct receivers that have fully pulled `ticket`; 0 if unknown.
    pub async fn transfer_count(&self, ticket: String) -> Result<u64, AtmanError> {
        let parsed = crate::BlobTicket::from_str(&ticket)
            .map_err(|e| AtmanError::InvalidTicket(e.to_string()))?;
        let (reply_sender, reply_receiver) = oneshot::channel();
        self.command_sender
            .lock()
            .await
            .send(Command::Blobs(
                crate::command::blobs::Command::FilesTransferCount {
                    hash: parsed.inner.hash(),
                    reply_sender,
                },
            ))
            .await
            .map_err(|_| AtmanError::ChannelClosed)?;
        Ok(reply_receiver
            .await
            .map_err(|_| AtmanError::ChannelClosed)?
            .unwrap_or(0))
    }
}

#[cfg(feature = "sync")]
#[uniffi::export(async_runtime = "tokio")]
impl AtmanClient {
    /// Connect to a remote node and sync the named doc.
    pub async fn connect_and_sync(
        &self,
        node_id: String,
        doc_space: String,
        doc_id: String,
    ) -> Result<(), AtmanError> {
        let node_id = iroh::EndpointId::from_str(&node_id)
            .map_err(|e| AtmanError::InvalidNodeId(e.to_string()))?;
        let (reply_sender, reply_receiver) = oneshot::channel();
        self.command_sender
            .lock()
            .await
            .send(Command::Sync(
                crate::command::sync::Command::ConnectAndSync {
                    node_id,
                    doc_space: doc_space.into(),
                    doc_id: doc_id.into(),
                    reply_sender,
                },
            ))
            .await
            .map_err(|_| AtmanError::ChannelClosed)?;
        reply_receiver
            .await
            .map_err(|_| AtmanError::ChannelClosed)?
            .map_err(|e| AtmanError::Internal(e.to_string()))?;
        info!("ConnectAndSync succeeded");
        Ok(())
    }

    /// Push a serialized update for `(doc_space, doc_id)` into local syncman.
    pub async fn sync_update(
        &self,
        doc_space: String,
        doc_id: String,
        data: Vec<u8>,
    ) -> Result<(), AtmanError> {
        let (msg, reply_receiver) = crate::sync_message::UpdateMessage {
            doc_space: doc_space.into(),
            doc_id: doc_id.into(),
            data,
        }
        .into();
        self.command_sender
            .lock()
            .await
            .send(Command::Sync(crate::command::sync::Command::Sync(
                Box::new(msg),
            )))
            .await
            .map_err(|_| AtmanError::ChannelClosed)?;
        reply_receiver
            .await
            .map_err(|_| AtmanError::ChannelClosed)?
            .map_err(|e| AtmanError::Internal(e.to_string()))?;
        info!("Sync update succeeded");
        Ok(())
    }

    /// Insert `data` into the list at `(collection_doc_id, property)` at `index`.
    pub async fn sync_list_insert(
        &self,
        doc_space: String,
        collection_doc_id: String,
        doc_id: String,
        property: String,
        data: Vec<u8>,
        index: u64,
    ) -> Result<(), AtmanError> {
        let (msg, reply_receiver) = crate::sync_message::ListInsertMessage {
            doc_space: doc_space.into(),
            collection_doc_id: collection_doc_id.into(),
            doc_id: doc_id.into(),
            property,
            data,
            index: index as usize,
        }
        .into();
        self.command_sender
            .lock()
            .await
            .send(Command::Sync(crate::command::sync::Command::Sync(
                Box::new(msg),
            )))
            .await
            .map_err(|_| AtmanError::ChannelClosed)?;
        reply_receiver
            .await
            .map_err(|_| AtmanError::ChannelClosed)?
            .map_err(|e| AtmanError::Internal(e.to_string()))?;
        info!("Sync list insert succeeded");
        Ok(())
    }

    /// Read `(doc_space, doc_id)` as JSON; `None` if not found.
    pub async fn sync_get(
        &self,
        doc_space: String,
        doc_id: String,
    ) -> Result<Option<String>, AtmanError> {
        let (msg, reply_receiver) = crate::sync_message::GetMessage {
            doc_space: doc_space.into(),
            doc_id: doc_id.into(),
        }
        .into();
        self.command_sender
            .lock()
            .await
            .send(Command::Sync(crate::command::sync::Command::Sync(
                Box::new(msg),
            )))
            .await
            .map_err(|_| AtmanError::ChannelClosed)?;
        match reply_receiver
            .await
            .map_err(|_| AtmanError::ChannelClosed)?
        {
            Ok(doc) => {
                let bytes = doc
                    .serialize()
                    .map_err(|e| AtmanError::Internal(e.to_string()))?;
                let json =
                    String::from_utf8(bytes).map_err(|e| AtmanError::InvalidUtf8(e.to_string()))?;
                Ok(Some(json))
            }
            Err(e) => {
                tracing::warn!(?e, "sync_get failed; returning None");
                Ok(None)
            }
        }
    }
}

fn init_tracing() {
    static ONCE: OnceCell<()> = OnceCell::new();
    ONCE.get_or_init(|| {
        // `try_init` so we don't fight a host-side subscriber (oslog etc.).
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::INFO)
            .with_ansi(false)
            .try_init();
    });
}
