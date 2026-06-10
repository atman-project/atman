//! UniFFI surface — the Swift / Kotlin entry point.
//!
//! Lives next to the legacy [`crate::binding`] C-ABI; both compile together
//! during the migration period so beam-ios keeps working off the C surface
//! while beam-android wires up against UniFFI. Once both consumers move,
//! `binding.rs` goes away.

#[cfg(feature = "blobs")]
use std::path::PathBuf;
#[cfg(any(feature = "blobs", feature = "sync"))]
use std::str::FromStr;
use std::sync::Arc;

use once_cell::sync::OnceCell;
use tokio::sync::{Mutex, mpsc, oneshot};
#[cfg(any(feature = "blobs", feature = "sync"))]
use tracing::info;

use crate::{Atman, Command, Config, actors::network, config::secret_key_from_hex};

/// All errors callers see across the FFI boundary. Strings are kept short
/// and machine-readable; richer context lives in tracing logs on the Rust
/// side.
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

/// One-per-app handle to a running Atman node. Constructed once on app
/// launch and held for the life of the process.
#[derive(uniffi::Object)]
pub struct AtmanClient {
    /// Holds the mpsc::Sender for commands the Atman event loop consumes.
    /// Wrapped in a Mutex so cloning across async calls stays sound under
    /// UniFFI's `Arc<Self>` receiver convention.
    command_sender: Mutex<mpsc::Sender<Command>>,
}

#[uniffi::export(async_runtime = "tokio")]
impl AtmanClient {
    /// Spin up the node and block until it's ready to accept commands.
    ///
    /// `custom_relay_url` is optional — passing `None` uses iroh's default
    /// relay set. The `sync_*` parameters only matter when the `sync`
    /// feature is enabled; pass an empty `syncman_dir` and `0` to disable
    /// the periodic-sync timer.
    #[uniffi::constructor]
    pub async fn new(
        identity_hex: String,
        network_key_hex: String,
        custom_relay_url: Option<String>,
        #[cfg(feature = "sync")] syncman_dir: String,
        #[cfg(feature = "sync")] sync_interval_secs: u64,
    ) -> Result<Arc<Self>, AtmanError> {
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

    // -------- Blobs --------

    /// Import one or more files into the blob store. Returns the shareable
    /// ticket the receiver will scan.
    #[cfg(feature = "blobs")]
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

    /// Pull every blob `ticket` references into `save_dir`, returning each
    /// saved file's path. The caller is responsible for moving files to
    /// their final destination (gallery, downloads, etc.).
    #[cfg(feature = "blobs")]
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

    /// How many distinct receivers have fully pulled `ticket`. Returns
    /// 0 if the ticket is unknown — same semantics as the legacy C-ABI.
    #[cfg(feature = "blobs")]
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

    // -------- Sync --------
    //
    // Mirrors the four sync entry points exposed by the legacy C-ABI:
    // connect-and-sync, sync-update, sync-list-insert, and sync-get. All
    // are gated behind the `sync` feature, same as `binding.rs`.

    /// Connect to a remote node and sync the named doc.
    #[cfg(feature = "sync")]
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

    /// Push a serialized update for `(doc_space, doc_id)` into the local
    /// syncman store. The receiver counterpart on a peer eventually picks
    /// it up via [`Self::connect_and_sync`].
    #[cfg(feature = "sync")]
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

    /// Insert `data` into the list at `(collection_doc_id, property)`
    /// at position `index`. Mirrors `send_atman_sync_list_insert_command`.
    #[cfg(feature = "sync")]
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

    /// Read the document at `(doc_space, doc_id)`. Returns the JSON-
    /// serialized form, or `None` if the document is not found.
    #[cfg(feature = "sync")]
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
                // `Document::serialize()` returns `Vec<u8>` of UTF-8 JSON;
                // the C-ABI wraps it in CString. Surface a real String here.
                let bytes = doc
                    .serialize()
                    .map_err(|e| AtmanError::Internal(e.to_string()))?;
                let json =
                    String::from_utf8(bytes).map_err(|e| AtmanError::InvalidUtf8(e.to_string()))?;
                Ok(Some(json))
            }
            Err(e) => {
                // Match the legacy C-ABI: an error path returns "no
                // document" rather than surfacing as an exception. Logged
                // server-side so it's still observable.
                tracing::warn!(?e, "sync_get failed; returning None");
                Ok(None)
            }
        }
    }
}

fn init_tracing() {
    static ONCE: OnceCell<()> = OnceCell::new();
    ONCE.get_or_init(|| {
        // `try_init` keeps us from racing with any host-side subscriber
        // the app may have already installed (e.g. tauri-plugin-log on
        // desktop or oslog on iOS).
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::INFO)
            .with_ansi(false)
            .try_init();
    });
}
