use ::iroh::NodeId;
use actman::Handle;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error, info};

#[cfg(feature = "rest")]
use crate::actors::rest;
#[cfg(feature = "rest")]
pub use crate::actors::rest::Config as RestConfig;
pub use crate::actors::{
    network::Config as NetworkConfig,
    sync::{Config as SyncConfig, message as sync_message},
};
use crate::{
    actors::{network, sync},
    discovery::{get_local_node_id, save_local_node_id},
    doc::{DocId, DocSpace},
};

mod actors;
pub mod binding;
pub mod config;
mod discovery;
pub use config::Config;
pub mod doc;

pub struct Atman {
    config: Config,
    command_receiver: mpsc::Receiver<Command>,
}

impl Atman {
    pub fn new(config: Config) -> Result<(Self, mpsc::Sender<Command>), Error> {
        let (command_sender, command_receiver) = mpsc::channel(100);
        Ok((
            Self {
                config,
                command_receiver,
            },
            command_sender,
        ))
    }

    pub async fn run(mut self, ready_sender: oneshot::Sender<Result<(), Error>>) {
        info!("Atman is running...");

        let mut runner = actman::Runner::new();

        let sync_actor = match sync::Actor::new(self.config.sync.clone()) {
            Ok(actor) => actor,
            Err(e) => {
                error!("Failed to create network actor: {e:?}");
                ready_sender
                    .send(Err(e.into()))
                    .expect("Failed to send ready signal");
                return;
            }
        };
        let sync_handle = runner.run(sync_actor);

        let network_actor =
            match network::Actor::new(&self.config.network, sync_handle.clone()).await {
                Ok(actor) => actor,
                Err(e) => {
                    error!("Failed to create network actor: {e:?}");
                    ready_sender
                        .send(Err(e.into()))
                        .expect("Failed to send ready signal");
                    return;
                }
            };
        let network_handle = runner.run(network_actor);

        #[cfg(feature = "rest")]
        let _rest_handle = runner.run(
            match rest::Actor::new(&self.config.rest, network_handle.clone()).await {
                Ok(actor) => actor,
                Err(e) => {
                    error!("Failed to create REST actor: {e:?}");
                    ready_sender
                        .send(Err(e.into()))
                        .expect("Failed to send ready signal");
                    return;
                }
            },
        );

        let local_node_id = get_local_node_id(&network_handle).await;
        if let Err(e) = save_local_node_id(local_node_id, &sync_handle).await {
            error!("Failed to save local node id: {e:?}");
            ready_sender
                .send(Err(e))
                .expect("Failed to send ready signal");
            return;
        }

        ready_sender
            .send(Ok(()))
            .expect("Failed to send ready signal");

        loop {
            if let Some(cmd) = self.command_receiver.recv().await {
                debug!("Command received: {:?}", cmd);
                match cmd {
                    Command::ConnectAndEcho {
                        node_id,
                        reply_sender,
                    } => {
                        network_handle
                            .send(network::Message::Echo {
                                node_id,
                                reply_sender,
                            })
                            .await
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
                            &network_handle,
                        )
                        .await
                    }
                    Command::Sync(msg) => sync_handle.send(msg).await,
                    Command::Status { reply_sender } => {
                        handle_status_command(reply_sender, &network_handle).await;
                    }
                    Command::Shutdown => {
                        runner.shutdown().await;
                        return;
                    }
                }
            }
        }
    }
}

async fn handle_connect_and_sync_command(
    node_id: NodeId,
    doc_space: DocSpace,
    doc_id: DocId,
    reply_sender: oneshot::Sender<Result<(), network::Error>>,
    network_handle: &Handle<network::Actor>,
) {
    let (nodes_sync_reply_sender, nodes_sync_reply_receiver) = oneshot::channel();
    network_handle
        .send(network::Message::Sync {
            node_id,
            doc_space: doc::protocol::DOC_SPACE.into(),
            doc_id: doc::protocol::nodes::DOC_ID.into(),
            reply_sender: nodes_sync_reply_sender,
        })
        .await;
    if let Err(e) = nodes_sync_reply_receiver.await {
        error!("failed to receive nodes sync reply. proceeding to handle sync command: {e:?}");
    }

    network_handle
        .send(network::Message::Sync {
            node_id,
            doc_space,
            doc_id,
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

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Network error: {0}")]
    Network(#[from] network::Error),
    #[error("Sync error: {0}")]
    Sync(#[from] sync::Error),
    #[cfg(feature = "rest")]
    #[error("Sync error: {0}")]
    Http(#[from] rest::Error),
    #[error("Document error: {0}")]
    Doc(#[from] doc::Error),
    #[error("Invalid config: {0}")]
    InvalidConfig(String),
    #[error("Unexpected document type")]
    UnexpectedDocumentType,
}

#[expect(
    clippy::large_enum_variant,
    reason = "Make AutomergeSyncHandle in sync::Message generic"
)]
#[derive(Debug)]
pub enum Command {
    ConnectAndEcho {
        node_id: NodeId,
        reply_sender: oneshot::Sender<Result<(), network::Error>>,
    },
    ConnectAndSync {
        node_id: NodeId,
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
    pub node_id: NodeId,
}
