use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error, info};

pub use crate::actors::network::Config as NetworkConfig;
#[cfg(feature = "rest")]
use crate::actors::rest;
#[cfg(feature = "rest")]
pub use crate::actors::rest::Config as RestConfig;
#[cfg(feature = "sync")]
pub use crate::actors::sync::{Config as SyncConfig, message as sync_message};
use crate::{actors::network, command::handle_command};
#[cfg(feature = "sync")]
use crate::{
    actors::sync,
    discovery::{get_local_node_id, save_local_node_id},
    periodic_sync::handle_sync_tick,
};

mod actors;
pub mod command;
pub mod uniffi_api;

// Emits the scaffolding that bridges proc-macro `#[uniffi::export]`
// items to the bindings generator. Must come AFTER the items the
// generator should pick up (declared via `pub mod uniffi_api`).
uniffi::setup_scaffolding!();
pub use command::Command;
pub mod config;
#[cfg(feature = "sync")]
mod discovery;
mod error;
#[cfg(feature = "sync")]
mod periodic_sync;
pub use error::Error;
#[cfg(feature = "sync")]
mod models;
pub use config::Config;
#[cfg(feature = "sync")]
pub mod doc;
#[cfg(feature = "blobs")]
pub use actors::network::{BlobTicket, BlobTicketError};

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

        #[cfg(feature = "sync")]
        let sync_handle = {
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
            runner.run(sync_actor)
        };

        let network_actor = match network::Actor::new(
            &self.config.network,
            #[cfg(feature = "sync")]
            sync_handle.clone(),
        )
        .await
        {
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
            match rest::Actor::new(
                &self.config.rest,
                network_handle.clone(),
                #[cfg(feature = "sync")]
                sync_handle.clone(),
            )
            .await
            {
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

        #[cfg(feature = "sync")]
        let local_node_id = {
            let local_node_id = get_local_node_id(&network_handle).await;
            if let Err(e) = save_local_node_id(local_node_id, &sync_handle).await {
                error!("Failed to save local node id: {e:?}");
                ready_sender
                    .send(Err(e))
                    .expect("Failed to send ready signal");
                return;
            }
            local_node_id
        };

        #[cfg(feature = "sync")]
        let mut sync_timer = self.config.sync_interval.map(tokio::time::interval);

        ready_sender
            .send(Ok(()))
            .expect("Failed to send ready signal");

        // `tokio::select!` doesn't accept `#[cfg]` on its arms, so we
        // pick a branch at compile time instead of trying to gate the
        // sync-timer arm inline.
        #[cfg(feature = "sync")]
        loop {
            tokio::select! {
                _ = async { sync_timer.as_mut().unwrap().tick().await }, if sync_timer.is_some() => {
                    debug!("Sync timer ticked");
                    match handle_sync_tick(&sync_handle, &network_handle, &local_node_id).await {
                        Ok(successful_syncs) => info!("periodic sync was successful with {successful_syncs} nodes"),
                        Err(e) => error!("error from periodic sync: {e}"),
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

        #[cfg(not(feature = "sync"))]
        while let Some(cmd) = self.command_receiver.recv().await {
            debug!("Command received: {:?}", cmd);
            if handle_command(cmd, &network_handle).await {
                info!("Shutting down Atman...");
                break;
            }
        }
    }
}
