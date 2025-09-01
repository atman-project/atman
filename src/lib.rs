use ::iroh::{NodeId, SecretKey};
use doc::{DocId, DocSpace, DocumentResolver};
use iroh::Iroh;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use syncman::{Syncman, automerge::AutomergeSyncman};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error, info};

use crate::doc::Document;

pub mod binding;
pub mod doc;
mod iroh;

pub struct Atman {
    config: Config,
    command_receiver: mpsc::Receiver<Command>,
    syncman: AutomergeSyncman,
    doc_resolver: DocumentResolver<AutomergeSyncman>,
}

impl Atman {
    pub fn new(config: Config) -> Result<(Self, mpsc::Sender<Command>), Error> {
        config.create_dirs()?;

        let path = config.syncman_path();
        let syncman = if !path.exists() || config.overwrite {
            let mut syncman = AutomergeSyncman::new();
            save_syncman(&mut syncman, &path)?;
            syncman
        } else {
            load_syncman(&path)?
        };

        let (command_sender, command_receiver) = mpsc::channel(100);
        Ok((
            Self {
                config,
                command_receiver,
                syncman,
                doc_resolver: DocumentResolver::new(),
            },
            command_sender,
        ))
    }

    pub async fn run(mut self, ready_sender: oneshot::Sender<Result<(), Error>>) {
        info!("Atman is running...");

        info!("Iroh is starting...");
        let iroh = match Iroh::new(self.config.iroh_key.clone()).await {
            Ok(iroh) => iroh,
            Err(e) => {
                ready_sender
                    .send(Err(e.into()))
                    .expect("Failed to send ready error signal");
                return;
            }
        };
        info!("Iroh started");

        ready_sender
            .send(Ok(()))
            .expect("Failed to send ready signal");

        loop {
            if let Some(cmd) = self.command_receiver.recv().await {
                debug!("Command received: {:?}", cmd);
                match cmd {
                    Command::ConnectAndEcho { node_id, .. } => {
                        if let Err(e) = iroh.connect(node_id).await {
                            error!("failed to connect: {e}");
                        }
                    }
                    Command::Sync(cmd) => {
                        info!("Command received: {cmd:?}");
                        match cmd {
                            SyncCommand::Update(SyncUpdateCommand {
                                doc_space,
                                doc_id,
                                data,
                            }) => match self.doc_resolver.deserialize(&doc_space, &doc_id, &data) {
                                Ok(doc) => {
                                    self.syncman.update(&doc);
                                    info!("Document updated in syncman");
                                    if let Err(e) = self.save() {
                                        error!("Failed to save atman: {e}");
                                    }
                                }
                                Err(e) => {
                                    error!("Failed to deserialize document: {e}");
                                    continue;
                                }
                            },
                            SyncCommand::ListInsert(SyncListInsertCommand {
                                doc_space,
                                doc_id,
                                property,
                                data,
                                index,
                            }) => match self.doc_resolver.deserialize(&doc_space, &doc_id, &data) {
                                Ok(doc) => {
                                    let obj_id = self
                                        .syncman
                                        .get_object_id(self.syncman.root(), property.clone());
                                    self.syncman.insert(obj_id, index, &doc);
                                    info!("Inserted into a list. prop:{property}, index:{index}");
                                    if let Err(e) = self.save() {
                                        error!("Failed to save atman: {e}");
                                    }
                                }
                                Err(e) => {
                                    error!("Failed to deserialize document: {e}");
                                    continue;
                                }
                            },
                            SyncCommand::Get {
                                cmd: SyncGetCommand { doc_space, doc_id },
                                reply_sender,
                            } => {
                                let doc = self
                                    .doc_resolver
                                    .hydrate(&doc_space, &doc_id, &self.syncman)
                                    .unwrap();
                                reply_sender.send(doc).unwrap();
                            }
                        }
                    }
                }
            }
        }
    }

    fn save(&mut self) -> Result<(), Error> {
        save_syncman(&mut self.syncman, &self.config.syncman_path())
    }
}

fn save_syncman(syncman: &mut AutomergeSyncman, path: &PathBuf) -> Result<(), Error> {
    let data = syncman.save();
    std::fs::write(path, data).map_err(|e| Error::IO {
        message: format!("Failed to write syncman file at {path:?}"),
        cause: e,
    })?;
    debug!("Syncman saved to {path:?}");
    Ok(())
}

fn load_syncman(path: &PathBuf) -> Result<AutomergeSyncman, Error> {
    let data = std::fs::read(path).map_err(|e| Error::IO {
        message: format!("Failed to read syncman file at {path:?}"),
        cause: e,
    })?;
    Ok(AutomergeSyncman::load(&data))
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
    #[error("IO error: {message}: {cause}")]
    IO {
        message: String,
        cause: std::io::Error,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub iroh_key: Option<SecretKey>,
    pub syncman_dir: PathBuf,
    pub overwrite: bool,
}

impl Config {
    fn create_dirs(&self) -> Result<(), Error> {
        std::fs::create_dir_all(&self.syncman_dir).map_err(|e| Error::IO {
            message: "Failed to create syncman directory".to_string(),
            cause: e,
        })?;
        info!("Created (or checked) syncman dir: {:?}", self.syncman_dir);
        Ok(())
    }

    fn syncman_path(&self) -> PathBuf {
        self.syncman_dir.join("syncman.dat")
    }
}

#[derive(Debug)]
pub enum Command {
    ConnectAndEcho { node_id: NodeId, payload: String },
    Sync(SyncCommand),
}

#[derive(Debug)]
pub enum SyncCommand {
    Update(SyncUpdateCommand),
    ListInsert(SyncListInsertCommand),
    Get {
        cmd: SyncGetCommand,
        reply_sender: oneshot::Sender<Document>,
    },
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncListInsertCommand {
    doc_space: DocSpace,
    doc_id: DocId,
    property: String,
    data: SerializedModel,
    index: usize,
}

impl From<SyncListInsertCommand> for Command {
    fn from(cmd: SyncListInsertCommand) -> Self {
        Command::Sync(SyncCommand::ListInsert(cmd))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncGetCommand {
    doc_space: DocSpace,
    doc_id: DocId,
}

impl From<SyncGetCommand> for (Command, oneshot::Receiver<Document>) {
    fn from(cmd: SyncGetCommand) -> Self {
        let (reply_sender, reply_receiver) = oneshot::channel();
        (
            Command::Sync(SyncCommand::Get { cmd, reply_sender }),
            reply_receiver,
        )
    }
}

type SerializedModel = Vec<u8>;

#[cfg(test)]
mod tests {
    use std::{
        env::temp_dir,
        thread::sleep,
        time::{Duration, SystemTime},
    };

    use uuid::Uuid;

    use crate::doc::{
        Document,
        aviation::{self, flight::Flight, flights::Flights},
    };

    use super::*;

    #[test]
    fn test_new() {
        let mut config = Config {
            iroh_key: None,
            syncman_dir: temp_dir().join("atman_test_syncman"),
            overwrite: false,
        };
        let _ = Atman::new(config.clone()).unwrap();
        // Check if the syncman has been saved.
        assert!(config.syncman_path().exists());
        let file_time = mtime(&config.syncman_path());

        // Recreate Atman with overwrite and check if the syncman file is updated.
        config.overwrite = true;
        sleep(Duration::from_millis(1)); // Ensure the mtime will be different.
        let _ = Atman::new(config.clone()).unwrap();
        assert!(config.syncman_path().exists());
        assert_ne!(mtime(&config.syncman_path()), file_time);
    }

    #[test_log::test(tokio::test)]
    async fn update() {
        let (atman, command_sender) = Atman::new(Config {
            iroh_key: None,
            syncman_dir: temp_dir().join("atman_test_syncman"),
            overwrite: false,
        })
        .unwrap();
        let (ready_sender, ready_receiver) = oneshot::channel();
        tokio::spawn(async move {
            atman.run(ready_sender).await;
        });
        ready_receiver
            .await
            .expect("ready channel shouldn't be closed")
            .expect("Atman should be ready successfully");

        let mut flights = Flights {
            flights: vec![Flight {
                id: Uuid::new_v4(),
                departure_airport: "ICN".into(),
                arrival_airport: "SEA".into(),
                departure_local_time: "2025-08-03T05:45:10Z".into(),
                arrival_local_time: "2025-08-03T05:45:10Z".into(),
                airline: "Korean Air".into(),
                aircraft: "A350-900".into(),
                flight_number: "KE1234".into(),
                booking_reference: "ABCDEF".into(),
            }],
        };
        let doc = Document::Flights(flights.clone());

        command_sender
            .send(
                SyncUpdateCommand {
                    doc_space: aviation::DOC_SPACE.into(),
                    doc_id: aviation::flights::DOC_ID.into(),
                    data: doc.serialize().unwrap(),
                }
                .into(),
            )
            .await
            .unwrap();

        let flight = Flight {
            id: Uuid::new_v4(),
            departure_airport: "LAX".into(),
            arrival_airport: "JFK".into(),
            departure_local_time: "2025-08-03T05:45:10Z".into(),
            arrival_local_time: "2025-08-03T05:45:10Z".into(),
            airline: "Delta Air Lines".into(),
            aircraft: "B777-300".into(),
            flight_number: "DL5678".into(),
            booking_reference: "GHIJKL".into(),
        };
        let doc = Document::Flight(flight.clone());

        command_sender
            .send(
                SyncListInsertCommand {
                    doc_space: aviation::DOC_SPACE.into(),
                    doc_id: aviation::flight::DOC_ID.into(),
                    property: "flights".into(),
                    index: 1,
                    data: doc.serialize().unwrap(),
                }
                .into(),
            )
            .await
            .unwrap();
        flights.flights.push(flight);

        let (cmd, reply_receiver) = SyncGetCommand {
            doc_space: aviation::DOC_SPACE.into(),
            doc_id: aviation::flights::DOC_ID.into(),
        }
        .into();
        command_sender.send(cmd).await.unwrap();
        let doc = reply_receiver.await.unwrap();
        assert_eq!(doc, Document::Flights(flights));
    }

    fn mtime(path: &PathBuf) -> SystemTime {
        std::fs::metadata(path).unwrap().modified().unwrap()
    }
}
