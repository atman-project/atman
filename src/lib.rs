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

        let syncman = AutomergeSyncman::new();
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

    pub async fn run(mut self) -> Result<(), Error> {
        info!("Atman is running...");

        info!("Iroh is starting...");
        let iroh = Iroh::new(self.config.iroh_key.clone()).await?;
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
                    Command::Sync(cmd) => {
                        info!("Command received: {cmd:?}");
                        match cmd {
                            SyncCommand::Update(SyncUpdateCommand {
                                doc_space,
                                doc_id,
                                data,
                            }) => {
                                let doc =
                                    self.doc_resolver.deserialize(&doc_space, &doc_id, &data)?;
                                self.syncman.update(&doc);
                                self.save_syncman()?;
                                info!("Document updated in syncman");
                            }
                            SyncCommand::ListInsert(SyncListInsertCommand {
                                doc_space,
                                doc_id,
                                property,
                                data,
                                index,
                            }) => {
                                let doc =
                                    self.doc_resolver.deserialize(&doc_space, &doc_id, &data)?;
                                let obj_id = self
                                    .syncman
                                    .get_object_id(self.syncman.root(), property.clone());
                                self.syncman.insert(obj_id, index, &doc);
                                self.save_syncman()?;
                                info!("Inserted into a list. prop:{property}, index:{index}")
                            }
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

    fn save_syncman(&mut self) -> Result<(), Error> {
        let path = self.config.syncman_dir.join("syncman.dat");
        let data = self.syncman.save();
        std::fs::write(&path, data)
            .map_err(|e| Error::InvalidConfig(format!("Failed to save syncman: {e}")))?;
        debug!("Syncman saved to {path:?}");
        Ok(())
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
    #[error("IO error: {message}: {cause}")]
    IO {
        message: String,
        cause: std::io::Error,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub iroh_key: Option<SecretKey>,
    pub syncman_dir: PathBuf,
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
    use uuid::Uuid;

    use crate::doc::{
        Document,
        aviation::{self, flight::Flight, flights::Flights},
    };

    use super::*;

    #[test_log::test(tokio::test)]
    async fn update() {
        let (atman, command_sender) = Atman::new(Config {
            iroh_key: None,
            syncman_dir: std::env::temp_dir().join("atman_test_syncman"),
        })
        .unwrap();
        tokio::spawn(async move {
            atman.run().await.unwrap();
        });

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
}
