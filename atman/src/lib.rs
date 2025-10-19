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
    command::handle_command,
    discovery::{get_local_node_id, save_local_node_id},
    sync_timer::handle_sync_tick,
};

mod actors;
pub mod binding;
mod command;
pub use command::Command;
pub mod config;
mod discovery;
mod error;
mod sync_timer;
pub use error::Error;
mod models;
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

        let mut sync_timer = self.config.sync_interval.map(tokio::time::interval);

        ready_sender
            .send(Ok(()))
            .expect("Failed to send ready signal");

        loop {
            tokio::select! {
                _ = async { sync_timer.as_mut().unwrap().tick().await }, if sync_timer.is_some() => {
                    debug!("Sync timer ticked");
                    if let Err(e) = handle_sync_tick(&sync_handle, &network_handle).await {
                        error!("error from periodic sync: {e}");
                    }
                },
                Some(cmd) = self.command_receiver.recv() => {
                    debug!("Command received: {:?}", cmd);
                    if handle_command(cmd, &network_handle, &sync_handle).await {
                        info!("Shutting down Atman...");
                        break;
                    }
                }
            }
        }
    }
}
