use ::iroh::NodeId;
use iroh::Iroh;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::{debug, error, info};

pub mod binding;
mod iroh;

pub struct Atman {
    command_receiver: mpsc::Receiver<Command>,
}

impl Atman {
    pub fn new() -> (Self, mpsc::Sender<Command>) {
        let (command_sender, command_receiver) = mpsc::channel(100);
        (Self { command_receiver }, command_sender)
    }

    pub async fn run(mut self) -> Result<(), Error> {
        info!("Atman is running...");

        info!("Iroh is starting...");
        let iroh = Iroh::new().await?;
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Command {
    ConnectAndEcho { node_id: NodeId, payload: String },
}
