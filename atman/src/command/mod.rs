#[cfg(feature = "blobs")]
pub mod blobs;
#[cfg(feature = "sync")]
pub mod sync;

use actman::Handle;
use iroh::EndpointId;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use tracing::error;

use crate::actors::network;

#[derive(Debug)]
pub enum Command {
    ConnectAndEcho {
        node_id: EndpointId,
        reply_sender: oneshot::Sender<Result<(), network::Error>>,
    },
    #[cfg(feature = "sync")]
    Sync(sync::Command),
    Status {
        reply_sender: oneshot::Sender<Status>,
    },
    #[cfg(feature = "blobs")]
    Blobs(blobs::Command),
    Shutdown,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Status {
    pub node_id: EndpointId,
}

pub async fn handle_command(
    command: Command,
    network_handle: &Handle<network::Actor>,
    #[cfg(feature = "sync")] sync_handle: &Handle<crate::actors::sync::Actor>,
) -> bool {
    match command {
        Command::ConnectAndEcho {
            node_id,
            reply_sender,
        } => {
            handle_connect_and_echo_command(node_id, reply_sender, network_handle).await;
        }
        #[cfg(feature = "sync")]
        Command::Sync(command) => {
            command.handle(network_handle, sync_handle).await;
        }
        Command::Status { reply_sender } => {
            handle_status_command(reply_sender, network_handle).await;
        }
        #[cfg(feature = "blobs")]
        Command::Blobs(command) => {
            command.handle(network_handle).await;
        }
        Command::Shutdown => {
            // Should shutdown
            return true;
        }
    }

    // Should not shutdown
    false
}

async fn handle_connect_and_echo_command(
    node_id: EndpointId,
    reply_sender: oneshot::Sender<Result<(), network::Error>>,
    network_handle: &Handle<network::Actor>,
) {
    network_handle
        .send(network::Message::ConnectAndEcho {
            node_id,
            reply_sender,
        })
        .await;
}

async fn handle_status_command(
    reply_sender: oneshot::Sender<Status>,
    network_handle: &Handle<network::Actor>,
) {
    let (network_status_sender, network_status_receiver) = oneshot::channel();
    network_handle
        .send(network::Message::Status {
            reply_sender: network_status_sender,
        })
        .await;

    let Ok(node_id) = network_status_receiver.await else {
        error!("Failed to receive network status");

        return;
    };

    let status = Status { node_id };
    let _ = reply_sender
        .send(status)
        .inspect_err(|_| error!("Failed to send status reply"));
}
