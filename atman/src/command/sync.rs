use actman::Handle;
use iroh::EndpointId;
use tokio::sync::oneshot;

use crate::{
    actors::{network, sync},
    doc::{DocId, DocSpace},
};

#[derive(Debug)]
pub enum Command {
    ConnectAndSync {
        node_id: EndpointId,
        doc_space: DocSpace,
        doc_id: DocId,
        reply_sender: oneshot::Sender<Result<(), network::Error>>,
    },
    Sync(Box<sync::message::Message>),
}

impl Command {
    pub async fn handle(
        self,
        network_handle: &Handle<network::Actor>,
        sync_handle: &Handle<sync::Actor>,
    ) {
        match self {
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
                handle_sync_command(*msg, sync_handle).await;
            }
        }
    }
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
