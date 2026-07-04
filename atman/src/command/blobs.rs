use std::{path::PathBuf, time::Duration};

use actman::Handle;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};

use crate::{BlobTicket, actors::network};

#[derive(Debug)]
pub enum Command {
    /// Import one or more files into the local blob store and
    /// return a shareable [`BlobTicket`].
    SendFiles {
        paths: Vec<PathBuf>,
        reply_sender: oneshot::Sender<Result<BlobTicket, network::Error>>,
    },
    /// Download the files described by `ticket` and export every contained file
    /// into `save_dir`. Returns one path per file written.
    ///
    /// `progress_sender` is fed [`DownloadProgress`] events as the transfer
    /// progresses; if you don't care about progress, pass a sender whose
    /// receiver you immediately drop.
    DownloadFiles {
        ticket: BlobTicket,
        save_dir: PathBuf,
        reply_sender: oneshot::Sender<Result<Vec<PathBuf>, network::Error>>,
        progress_sender: mpsc::Sender<DownloadProgress>,
    },
    /// Returns how many distinct receivers have fully pulled the
    /// ticket whose hash is `hash`.
    FilesTransferCount {
        hash: iroh_blobs::Hash,
        reply_sender: oneshot::Sender<Option<u64>>,
    },
}

impl Command {
    pub async fn handle(self, network_handle: &Handle<network::Actor>) {
        match self {
            Command::SendFiles {
                paths,
                reply_sender,
            } => {
                network_handle
                    .send(network::Message::SendFiles {
                        paths,
                        reply_sender,
                    })
                    .await;
            }
            Command::DownloadFiles {
                ticket,
                save_dir,
                reply_sender,
                progress_sender,
            } => {
                network_handle
                    .send(network::Message::DownloadFiles {
                        ticket,
                        save_dir,
                        reply_sender,
                        progress_sender,
                    })
                    .await;
            }
            Command::FilesTransferCount { hash, reply_sender } => {
                network_handle
                    .send(network::Message::FilesTransferCount { hash, reply_sender })
                    .await;
            }
        }
    }
}

/// Streaming progress events for [`Command::DownloadFiles`].
///
/// Informational only — the canonical success / failure signal is
/// still the [`Command::DownloadFiles.reply_sender`].
///
/// `path` is the string form of [`PathBuf`] so this type can cross the
/// UniFFI boundary unchanged (PathBuf isn't a UniFFI built-in).
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
pub enum DownloadProgress {
    /// Cumulative payload bytes received from the sender so far. Fires
    /// many times during a transfer; the value monotonically increases.
    Bytes { downloaded: u64 },
    /// All bytes have been received.
    /// Carries the iroh-blobs `Stats` flattened out, so the consumer doesn't
    /// need to depend on iroh-blobs types directly.
    FetchDone {
        /// Payload (file) bytes received from the wire.
        payload_bytes_read: u64,
        /// Framing / handshake bytes received from the wire.
        other_bytes_read: u64,
        /// Wall-clock time from start of fetch to completion.
        elapsed: Duration,
    },
    /// One file inside the ticket has been written to disk.
    FileExported { path: String },
}
