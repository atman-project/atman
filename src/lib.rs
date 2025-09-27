use ::iroh::NodeId;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error, info};

use crate::actors::{network, sync};
pub use crate::actors::{network::Config as NetworkConfig, sync::Config as SyncConfig};

mod actors;
pub mod binding;
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

        ready_sender
            .send(Ok(()))
            .expect("Failed to send ready signal");

        loop {
            if let Some(cmd) = self.command_receiver.recv().await {
                debug!("Command received: {:?}", cmd);
                match cmd {
                    Command::ConnectAndEcho { node_id, .. } => {
                        network_handle.send(network::Message::Echo(node_id)).await
                    }
                    Command::ConnectAndSync { node_id } => {
                        network_handle.send(network::Message::Sync(node_id)).await
                    }
                    Command::Sync(msg) => sync_handle.send(msg).await,
                }
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Network actor error: {0}")]
    NetworkActorError(#[from] network::Error),
    #[error("Sync actor error: {0}")]
    SyncActorError(#[from] sync::Error),
    #[error("Double initialization: {0}")]
    DoubleInit(String),
    #[error("Invalid config: {0}")]
    InvalidConfig(String),
    #[error("Resolver error: {0}")]
    Resolver(#[from] doc::Error),
    #[error("IO error: {message}: {cause}")]
    IO {
        message: String,
        cause: std::io::Error,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub network: network::Config,
    pub sync: sync::Config,
}

#[expect(
    clippy::large_enum_variant,
    reason = "Make AutomergeSyncHandle in sync::Message generic"
)]
#[derive(Debug)]
pub enum Command {
    ConnectAndEcho { node_id: NodeId, payload: String },
    ConnectAndSync { node_id: NodeId },
    Sync(sync::Message),
}
