use ::iroh::{NodeId, SecretKey};
use doc::{DocId, DocResolver, DocSpace, Resolver as _};
use iroh::Iroh;
use serde::{Deserialize, Serialize};
use syncman::{Syncman, automerge::AutomergeSyncman};
use tokio::sync::mpsc;
use tracing::{debug, error, info};

pub mod binding;
pub mod doc;
mod iroh;

pub struct Atman {
    config: Config,
    command_receiver: mpsc::Receiver<Command>,
    syncman: AutomergeSyncman,
}

impl Atman {
    pub fn new(config: Config) -> (Self, mpsc::Sender<Command>) {
        let syncman = AutomergeSyncman::new();
        let (command_sender, command_receiver) = mpsc::channel(100);
        (
            Self {
                config,
                command_receiver,
                syncman,
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
                    Command::Sync(cmd) => match cmd {
                        SyncCommand::Update(SyncUpdateCommand {
                            doc_space,
                            doc_id,
                            data,
                        }) => {
                            info!("Syncing update for {doc_space:?}: {doc_id:?}: {data:?}",);
                            let flight = DocResolver::deserialize(&doc_space, &doc_id, &data)?;
                            self.syncman.update(&flight);
                            info!("Flight updated in syncman");
                        }
                    },
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
    #[error("Resolver error: {0}")]
    Resolver(#[from] doc::Error),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub iroh_key: Option<SecretKey>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Command {
    ConnectAndEcho { node_id: NodeId, payload: String },
    Sync(SyncCommand),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SyncCommand {
    Update(SyncUpdateCommand),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncUpdateCommand {
    doc_space: DocSpace,
    doc_id: DocId,
    data: SerializedModel,
}

impl From<SyncUpdateCommand> for Command {
    fn from(cmd: SyncUpdateCommand) -> Self {
        Command::Sync(SyncCommand::Update(cmd))
    }
}

type SerializedModel = Vec<u8>;
