use std::str::FromStr;

use atman::{Atman, Error};
use clap::Parser;
use iroh::NodeId;
use tracing::{error, info};
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

async fn run(args: Args) -> Result<(), Error> {
    let config = args.to_config()?;

    let (atman, command_sender) = Atman::new(config);
    let atman_task = tokio::spawn(async move {
        if let Err(e) = atman.run().await {
            error!("Error from Atman: {e}");
        }
    });

    if let Some(command) = args.command {
        match command {
            Command::ConnectAndEcho { node_id, payload } => {
                info!("Connecting to node: {node_id} with payload: {payload}");
                if let Err(e) = command_sender
                    .send(atman::Command::ConnectAndEcho { node_id, payload })
                    .await
                {
                    error!("Channel send error: {e}");
                }
            }
        }
    }

    if let Err(e) = atman_task.await {
        error!("Failed to wait until Atman is terminated: {e}");
    }
    Ok(())
}

#[derive(Debug, Parser)]
struct Args {
    #[clap(long)]
    iroh_key: Option<String>,
    #[clap(subcommand)]
    command: Option<Command>,
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

        Ok(atman::Config { iroh_key })
    }
}

#[derive(Debug, Parser)]
enum Command {
    ConnectAndEcho { node_id: NodeId, payload: String },
}
