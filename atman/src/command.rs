use actman::Handle;
use iroh::EndpointId;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use tracing::error;

use crate::{
    actors::{network, sync},
    doc::{DocId, DocSpace},
};

#[expect(
    clippy::large_enum_variant,
    reason = "Make AutomergeSyncHandle in sync::Message generic"
)]
#[derive(Debug)]
pub enum Command {
    ConnectAndEcho {
        node_id: EndpointId,
        reply_sender: oneshot::Sender<Result<(), network::Error>>,
    },
    ConnectAndSync {
        node_id: EndpointId,
        doc_space: DocSpace,
        doc_id: DocId,
        reply_sender: oneshot::Sender<Result<(), network::Error>>,
    },
    Sync(sync::message::Message),
    Status {
        reply_sender: oneshot::Sender<Status>,
    },
    Shutdown,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Status {
    pub node_id: EndpointId,
}

pub async fn handle_command(
    command: Command,
    network_handle: &Handle<network::Actor>,
    sync_handle: &Handle<sync::Actor>,
) -> bool {
    match command {
        Command::ConnectAndEcho {
            node_id,
            reply_sender,
        } => {
            handle_connect_and_echo_command(node_id, reply_sender, network_handle).await;
        }
        Command::ConnectAndSync {
            node_id,
            doc_space,
            doc_id,
            reply_sender,
        } => {
            handle_connect_and_sync_command(
                node_id,
                doc_space,
                doc_id,
                reply_sender,
                network_handle,
            )
            .await;
        }
        Command::Sync(msg) => {
            handle_sync_command(msg, sync_handle).await;
        }
        Command::Status { reply_sender } => {
            handle_status_command(reply_sender, network_handle).await;
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

async fn handle_connect_and_sync_command(
    node_id: EndpointId,
    doc_space: DocSpace,
    doc_id: DocId,
    reply_sender: oneshot::Sender<Result<(), network::Error>>,
    network_handle: &Handle<network::Actor>,
) {
    network_handle
        .send(network::Message::ConnectAndSync {
            node_id,
            doc_space,
            doc_id,
            reply_sender,
        })
        .await;
}

async fn handle_sync_command(msg: sync::message::Message, sync_handle: &Handle<sync::Actor>) {
    sync_handle.send(msg).await;
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
