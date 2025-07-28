use ::iroh::{NodeId, SecretKey};
use iroh::Iroh;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::{debug, error, info};

pub mod binding;
mod iroh;

pub struct Atman {
    config: Config,
    command_receiver: mpsc::Receiver<Command>,
}

impl Atman {
    pub fn new(config: Config) -> (Self, mpsc::Sender<Command>) {
        let (command_sender, command_receiver) = mpsc::channel(100);
        (
            Self {
                config,
                command_receiver,
            },
            command_sender,
        )
    }

    pub async fn run(mut self) -> Result<(), Error> {
        info!("Atman is running...");

        info!("Iroh is starting...");
        let iroh = Iroh::new(self.config.iroh_key).await?;
        info!("Iroh started");

        loop {
            if let Some(cmd) = self.command_receiver.recv().await {
                debug!("Command received: {:?}", cmd);
                match cmd {
                    Command::ConnectAndEcho { node_id, .. } => {
                        if let Err(e) = iroh.connect(node_id).await {
                            error!("failed to connect: {e}");
                        }
                    }
                }
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Iroh error: {0}")]
    Iroh(#[from] iroh::Error),
    #[error("Double initialization: {0}")]
    DoubleInit(String),
    #[error("Invalid config: {0}")]
    InvalidConfig(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub iroh_key: Option<SecretKey>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Command {
    ConnectAndEcho { node_id: NodeId, payload: String },
}
