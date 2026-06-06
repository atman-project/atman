use std::path::PathBuf;

use actman::Handle;
use tokio::sync::oneshot;

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
    DownloadFiles {
        ticket: BlobTicket,
        save_dir: PathBuf,
        reply_sender: oneshot::Sender<Result<Vec<PathBuf>, network::Error>>,
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
            } => {
                network_handle
                    .send(network::Message::DownloadFiles {
                        ticket,
                        save_dir,
                        reply_sender,
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
