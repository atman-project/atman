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

#[derive(Debug, Parser)]
struct Args {
    #[clap(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Parser)]
enum Command {
    ConnectAndEcho { node_id: NodeId, payload: String },
}

async fn run(args: Args) -> Result<(), Error> {
    let (atman, command_sender) = Atman::new();
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
