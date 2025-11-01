use std::{net::SocketAddr, path::PathBuf, str::FromStr, time::Duration};

use atman::{
    Atman, Error, NetworkConfig, RestConfig, SyncConfig,
    config::secret_key_from_hex,
    doc::{DocId, DocSpace},
    sync_message,
};
use clap::Parser;
use iroh::EndpointId;
use qrcode::{QrCode, QrResult, render::unicode};
use tokio::{
    signal,
    sync::{mpsc, oneshot},
};
use tracing::{debug, error, info};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
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

    match args.command {
        Command::Daemonize => {
            handle_status(&command_sender).await;
            daemonize().await;
        }
        Command::Status => {
            handle_status(&command_sender).await;
        }
        Command::ConnectAndEcho { node_id } => {
            handle_connect_and_echo(&command_sender, node_id).await;
        }
        Command::ConnectAndSync {
            node_id,
            doc_space,
            doc_id,
        } => {
            handle_connect_and_sync(&command_sender, node_id, doc_space, doc_id).await;
        }
        Command::GetDocument { doc_space, doc_id } => {
            handle_get_document(&command_sender, doc_space, doc_id).await;
        }
    }

    // Shutdown Atman.
    command_sender
        .send(atman::Command::Shutdown)
        .await
        .inspect_err(|e| {
            error!("Channel send error: {e}");
        })?;
    info!("Waiting for Atman to terminate...");
    if let Err(e) = atman_task.await {
        error!("Failed to wait until Atman is terminated: {e}");
    }
    Ok(())
}

/// A future that resolves when a termination signal is received.
async fn daemonize() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    info!("Termination signal received");
}

async fn handle_status(command_sender: &mpsc::Sender<atman::Command>) {
    info!("Handling status command");
    let (reply_sender, reply_receiver) = oneshot::channel();
    if let Err(e) = command_sender
        .send(atman::Command::Status { reply_sender })
        .await
    {
        error!("Channel send error: {e}");
        return;
    }
    let Ok(status) = reply_receiver.await else {
        error!("Failed to receive status reply");
        return;
    };

    println!("============================");
    println!(" Status");
    println!("============================");
    println!(
        "{}",
        serde_json::to_string_pretty(&status).expect("Status should be serializable")
    );

    match generate_qr(
        serde_json::to_string(&status)
            .expect("Status should be serializable")
            .as_bytes(),
    ) {
        Ok(code) => {
            println!("{code}");
        }
        Err(e) => {
            error!("Failed to generate QR code: {e}");
        }
    }
}

fn generate_qr(data: &[u8]) -> QrResult<String> {
    let image = QrCode::new(data)?
        .render::<unicode::Dense1x2>()
        .quiet_zone(true)
        .module_dimensions(1, 1)
        .build();
    Ok(image)
}

async fn handle_connect_and_echo(
    command_sender: &mpsc::Sender<atman::Command>,
    node_id: EndpointId,
) {
    info!("Connecting to node {node_id} to echo");
    let (reply_sender, reply_receiver) = oneshot::channel();
    if let Err(e) = command_sender
        .send(atman::Command::ConnectAndEcho {
            node_id,
            reply_sender,
        })
        .await
    {
        error!("Channel send error: {e}");
        return;
    }

    match reply_receiver.await {
        Ok(Ok(())) => info!("Successfully connected and echoed"),
        Ok(Err(e)) => error!("Failed to connect and echo: {e:?}"),
        Err(e) => error!("Failed to receive reply: {e:?}"),
    }
}

async fn handle_connect_and_sync(
    command_sender: &mpsc::Sender<atman::Command>,
    node_id: EndpointId,
    doc_space: DocSpace,
    doc_id: DocId,
) {
    info!("Connecting to node {node_id} to sync");
    let (reply_sender, reply_receiver) = oneshot::channel();
    if let Err(e) = command_sender
        .send(atman::Command::ConnectAndSync {
            node_id,
            doc_space,
            doc_id,
            reply_sender,
        })
        .await
    {
        error!("Channel send error: {e}");
        return;
    }

    match reply_receiver.await {
        Ok(Ok(())) => info!("Successfully connected and synced"),
        Ok(Err(e)) => error!("Failed to connect and sync: {e:?}"),
        Err(e) => error!("Failed to receive reply: {e:?}"),
    }
}

async fn handle_get_document(
    command_sender: &mpsc::Sender<atman::Command>,
    doc_space: DocSpace,
    doc_id: DocId,
) {
    info!("Getting a document: {doc_space:?}/{doc_id:?}");
    let (reply_sender, reply_receiver) = oneshot::channel();
    if let Err(e) = command_sender
        .send(atman::Command::Sync(sync_message::Message::Get {
            msg: sync_message::GetMessage { doc_space, doc_id },
            reply_sender,
        }))
        .await
    {
        error!("Channel send error: {e}");
        return;
    }

    match reply_receiver.await {
        Ok(Ok(doc)) => {
            debug!("Retrieved {doc:?}");
            match doc.serialize_pretty() {
                Ok(json) => println!("{json}"),
                Err(e) => error!("Failed to serialize document: {e:?}"),
            }
        }
        Ok(Err(e)) => error!("Failed to get document: {e:?}"),
        Err(e) => error!("Failed to receive reply: {e:?}"),
    }
}

#[derive(Debug, Parser)]
struct Args {
    #[clap(long)]
    identity: String,
    #[clap(long)]
    network_key: Option<String>,
    #[clap(long)]
    rest_addr: Option<SocketAddr>,
    #[clap(long)]
    syncman_dir: String,
    #[clap(long, value_parser = humantime::parse_duration)]
    sync_interval: Option<Duration>,
    #[clap(subcommand)]
    command: Command,
}

impl Args {
    fn to_config(&self) -> Result<atman::Config, Error> {
        let identity = secret_key_from_hex(self.identity.as_bytes())
            .map_err(|e| Error::InvalidConfig(e.to_string()))?;

        let network_key = match &self.network_key {
            Some(key) => Some(
                iroh::SecretKey::from_str(key.as_str())
                    .map_err(|_| Error::InvalidConfig("Invalid Iroh key".to_string()))?,
            ),
            None => None,
        };

        Ok(atman::Config {
            identity,
            network: NetworkConfig { key: network_key },
            sync: SyncConfig {
                syncman_dir: PathBuf::from(&self.syncman_dir),
            },
            sync_interval: self.sync_interval,
            rest: self
                .rest_addr
                .map_or(Default::default(), |addr| RestConfig { addr }),
        })
    }
}

#[derive(Debug, Parser)]
enum Command {
    Daemonize,
    Status,
    ConnectAndEcho {
        node_id: EndpointId,
    },
    ConnectAndSync {
        node_id: EndpointId,
        doc_space: DocSpace,
        doc_id: DocId,
    },
    GetDocument {
        doc_space: DocSpace,
        doc_id: DocId,
    },
}
