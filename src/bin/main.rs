use std::{path::PathBuf, str::FromStr};

use atman::{
    Atman, Error, NetworkConfig, SyncConfig,
    doc::{DocId, DocSpace},
    sync_message,
};
use clap::Parser;
use iroh::NodeId;
use tokio::{signal, sync::oneshot};
use tracing::{debug, error, info};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_max_level(tracing::Level::INFO)
        .init();

    let args = Args::parse();

    info!("Starting Atman binary...");
    if let Err(e) = run(args).await {
        error!("Error: {e:?}");
    } else {
        info!("Atman has been terminated.");
    }
}

async fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let config = args.to_config()?;

    let (atman, command_sender) = Atman::new(config)?;
    let (ready_sender, ready_receiver) = oneshot::channel();
    let atman_task = tokio::spawn(async move { atman.run(ready_sender).await });
    ready_receiver
        .await
        .expect("ready channel shouldn't be closed")?;

    if let Some(command) = args.command {
        match command {
            Command::ConnectAndEcho { node_id, payload } => {
                info!("Connecting to node {node_id} with payload: {payload}");
                command_sender
                    .send(atman::Command::ConnectAndEcho { node_id, payload })
                    .await
                    .inspect_err(|e| {
                        error!("Channel send error: {e}");
                    })?;
            }
            Command::ConnectAndSync { node_id } => {
                info!("Connecting to node {node_id} to sync");
                command_sender
                    .send(atman::Command::ConnectAndSync { node_id })
                    .await
                    .inspect_err(|e| {
                        error!("Channel send error: {e}");
                    })?;
            }
            Command::GetDocument { doc_space, doc_id } => {
                info!("Getting a document: {doc_space:?}/{doc_id:?}");
                let (reply_sender, reply_receiver) = oneshot::channel();
                command_sender
                    .send(atman::Command::Sync(sync_message::Message::Get {
                        msg: sync_message::GetMessage { doc_space, doc_id },
                        reply_sender,
                    }))
                    .await
                    .inspect_err(|e| {
                        error!("Channel send error: {e}");
                    })?;
                let doc = reply_receiver.await.inspect_err(|e| {
                    error!("Failed to receive document: {e:?}");
                })?;
                debug!("Retrieved {doc:?}");
                let json = doc.serialize_pretty()?;
                println!("{json}");
            }
        }
    }

    // Wait for Ctrl+C signal.
    let _ = signal::ctrl_c().await;

    // Shutdown Atman.
    command_sender
        .send(atman::Command::Shutdown)
        .await
        .inspect_err(|e| {
            error!("Channel send error: {e}");
        })?;
    if let Err(e) = atman_task.await {
        error!("Failed to wait until Atman is terminated: {e}");
    }
    Ok(())
}

#[derive(Debug, Parser)]
struct Args {
    #[clap(long)]
    iroh_key: Option<String>,
    #[clap(long)]
    syncman_dir: String,
    #[clap(subcommand)]
    command: Option<Command>,
    #[clap(long, default_value_t = false)]
    overwrite: bool,
}

impl Args {
    fn to_config(&self) -> Result<atman::Config, Error> {
        let iroh_key = match &self.iroh_key {
            Some(key) => Some(
                iroh::SecretKey::from_str(key.as_str())
                    .map_err(|_| Error::InvalidConfig("Invalid Iroh key".to_string()))?,
            ),
            None => None,
        };

        Ok(atman::Config {
            network: NetworkConfig { key: iroh_key },
            sync: SyncConfig {
                syncman_dir: PathBuf::from(&self.syncman_dir),
                overwrite: self.overwrite,
            },
        })
    }
}

#[derive(Debug, Parser)]
enum Command {
    ConnectAndEcho { node_id: NodeId, payload: String },
    ConnectAndSync { node_id: NodeId },
    GetDocument { doc_space: DocSpace, doc_id: DocId },
}
