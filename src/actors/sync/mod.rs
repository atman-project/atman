pub mod message;

use std::{fmt::Debug, io, path::PathBuf};

use serde::{Deserialize, Serialize};
use syncman::{
    Syncman,
    automerge::{AutomergeSyncHandle, AutomergeSyncman},
};
use tokio::sync::oneshot;
use tracing::{error, info, warn};

use crate::{
    doc::{self, Document, DocumentResolver},
    sync::message::Message,
    sync_message::{GetMessage, ListInsertMessage, UpdateMessage},
};

pub struct Actor {
    config: Config,
    syncman: AutomergeSyncman,
    doc_resolver: DocumentResolver<AutomergeSyncman>,
}

#[async_trait::async_trait]
impl actman::Actor for Actor {
    type Message = Message;

    async fn run(mut self, mut state: actman::State<Self>) {
        loop {
            tokio::select! {
                Some(message) = state.message_receiver.recv() => {
                    self.handle_message(message)
                }
                Some(ctrl) = state.control_receiver.recv() => {
                    match ctrl {
                        actman::Control::Shutdown => {
                            info!("Actor received shutdown control.");
                            return;
                        },
                    }
                }
                else => {
                    warn!("All channels closed, terminating actor.");
                    return;
                }
            }
        }
    }
}

impl Actor {
    pub fn new(config: Config) -> Result<Self, Error> {
        config.create_dirs()?;

        let path = config.syncman_path();
        let syncman = if !path.exists() || config.overwrite {
            let mut syncman = AutomergeSyncman::new();
            save_syncman(&mut syncman, &path)?;
            syncman
        } else {
            load_syncman(&path)?
        };

        Ok(Self {
            config,
            syncman,
            doc_resolver: DocumentResolver::new(),
        })
    }

    fn handle_message(&mut self, message: Message) {
        match message {
            Message::Update { msg, reply_sender } => self.handle_update_message(msg, reply_sender),
            Message::ListInsert { msg, reply_sender } => {
                self.handle_list_insert_message(msg, reply_sender)
            }

            Message::Get { msg, reply_sender } => self.handle_get_message(msg, reply_sender),
            Message::InitiateSync { reply_sender } => {
                self.handle_initiate_sync_message(reply_sender)
            }
            Message::ApplySync {
                data,
                handle,
                reply_sender,
            } => self.handle_apply_sync_message(data, handle, reply_sender),
        }
    }

    fn handle_update_message(
        &mut self,
        msg: UpdateMessage,
        reply_sender: oneshot::Sender<Result<(), Error>>,
    ) {
        let _ = reply_sender
            .send(
                self.handle_update_message_inner(msg)
                    .inspect_err(|e| error!("Failed to handle update message: {e:?}")),
            )
            .inspect_err(|_| error!("Failed to send reply"));
    }

    fn handle_update_message_inner(
        &mut self,
        UpdateMessage {
            doc_space,
            doc_id,
            data,
        }: UpdateMessage,
    ) -> Result<(), Error> {
        let doc = self.doc_resolver.deserialize(&doc_space, &doc_id, &data)?;
        self.syncman.update(&doc);
        info!("Document updated in syncman");
        self.save()
    }

    fn handle_list_insert_message(
        &mut self,
        msg: ListInsertMessage,
        reply_sender: oneshot::Sender<Result<(), Error>>,
    ) {
        let _ = reply_sender
            .send(
                self.handle_list_insert_message_inner(msg)
                    .inspect_err(|e| error!("Failed to handle list insert message: {e:?}")),
            )
            .inspect_err(|_| error!("Failed to send reply"));
    }

    fn handle_list_insert_message_inner(
        &mut self,
        ListInsertMessage {
            doc_space,
            doc_id,
            property,
            data,
            index,
        }: ListInsertMessage,
    ) -> Result<(), Error> {
        let doc = self.doc_resolver.deserialize(&doc_space, &doc_id, &data)?;
        let obj_id = self
            .syncman
            .get_object_id(self.syncman.root(), property.clone());
        self.syncman.insert(obj_id, index, &doc);
        info!("Inserted into a list. prop:{property}, index:{index}");
        self.save()
    }

    fn handle_get_message(
        &self,
        GetMessage { doc_space, doc_id }: GetMessage,
        reply_sender: oneshot::Sender<Result<Document, Error>>,
    ) {
        let _ = reply_sender
            .send(
                self.doc_resolver
                    .hydrate(&doc_space, &doc_id, &self.syncman)
                    .map_err(|e| {
                        error!("Failed to handle get message: {e:?}");
                        e.into()
                    }),
            )
            .inspect_err(|_| error!("Failed to send reply"));
    }

    fn handle_initiate_sync_message(&mut self, reply_sender: oneshot::Sender<AutomergeSyncHandle>) {
        let _ = reply_sender
            .send(self.syncman.initiate_sync())
            .inspect_err(|_| error!("Failed to send reply"));
    }

    fn handle_apply_sync_message(
        &mut self,
        data: Vec<u8>,
        handle: AutomergeSyncHandle,
        reply_sender: oneshot::Sender<Result<AutomergeSyncHandle, Error>>,
    ) {
        let _ = reply_sender
            .send(
                self.handle_apply_sync_message_inner(data, handle)
                    .inspect_err(|e| error!("Failed to handle apply sync message: {e:?}")),
            )
            .inspect_err(|_| error!("Failed to send reply"));
    }

    fn handle_apply_sync_message_inner(
        &mut self,
        data: Vec<u8>,
        mut handle: AutomergeSyncHandle,
    ) -> Result<AutomergeSyncHandle, Error> {
        self.syncman.apply_sync(&mut handle, &data);
        self.save()?;
        Ok(handle)
    }

    fn save(&mut self) -> Result<(), Error> {
        save_syncman(&mut self.syncman, &self.config.syncman_path())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
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

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("IO error: {message}: {cause}")]
    IO { message: String, cause: io::Error },
    #[error("Document error: {0}")]
    Document(#[from] doc::Error),
}

fn save_syncman(syncman: &mut AutomergeSyncman, path: &PathBuf) -> Result<(), Error> {
    let data = syncman.save();
    std::fs::write(path, data).map_err(|e| Error::IO {
        message: format!("Failed to write syncman file at {path:?}"),
        cause: e,
    })?;
    info!("Syncman saved to {path:?}");
    Ok(())
}

fn load_syncman(path: &PathBuf) -> Result<AutomergeSyncman, Error> {
    let data = std::fs::read(path).map_err(|e| Error::IO {
        message: format!("Failed to read syncman file at {path:?}"),
        cause: e,
    })?;
    Ok(AutomergeSyncman::load(&data))
}

#[cfg(test)]
mod tests {
    use std::{
        env::temp_dir,
        thread::sleep,
        time::{Duration, SystemTime},
    };

    use actman::{Actor as _, Control, State};
    use tokio::sync::mpsc;
    use uuid::Uuid;

    use super::*;
    use crate::doc::{
        Document,
        aviation::{self, flight::Flight, flights::Flights},
    };

    #[test]
    fn test_new() {
        let mut config = Config {
            syncman_dir: temp_dir().join("atman_test_syncman"),
            overwrite: false,
        };
        let _actor = Actor::new(config.clone()).unwrap();
        // Check if the syncman has been saved.
        assert!(config.syncman_path().exists());
        let file_time = mtime(&config.syncman_path());

        // Recreate Atman with overwrite and check if the syncman file is updated.
        config.overwrite = true;
        sleep(Duration::from_millis(1)); // Ensure the mtime will be different.
        let _actor = Actor::new(config.clone()).unwrap();
        assert!(config.syncman_path().exists());
        assert_ne!(mtime(&config.syncman_path()), file_time);
    }

    #[test_log::test(tokio::test)]
    async fn update() {
        let actor = Actor::new(Config {
            syncman_dir: temp_dir().join("atman_test_syncman"),
            overwrite: false,
        })
        .unwrap();
        let (state, message_sender, _control_sender) = actman_state::<Actor>();
        tokio::spawn(async move {
            actor.run(state).await;
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

        let (msg, reply_receiver) = UpdateMessage {
            doc_space: aviation::DOC_SPACE.into(),
            doc_id: aviation::flights::DOC_ID.into(),
            data: doc.serialize().unwrap(),
        }
        .into();
        message_sender.send(msg).await.unwrap();
        reply_receiver.await.unwrap().unwrap();

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

        let (msg, reply_receiver) = ListInsertMessage {
            doc_space: aviation::DOC_SPACE.into(),
            doc_id: aviation::flight::DOC_ID.into(),
            property: "flights".into(),
            index: 1,
            data: doc.serialize().unwrap(),
        }
        .into();
        message_sender.send(msg).await.unwrap();
        reply_receiver.await.unwrap().unwrap();
        flights.flights.push(flight);

        let (cmd, reply_receiver) = GetMessage {
            doc_space: aviation::DOC_SPACE.into(),
            doc_id: aviation::flights::DOC_ID.into(),
        }
        .into();
        message_sender.send(cmd).await.unwrap();
        let doc = reply_receiver.await.unwrap().unwrap();
        assert_eq!(doc, Document::Flights(flights));
    }

    fn mtime(path: &PathBuf) -> SystemTime {
        std::fs::metadata(path).unwrap().modified().unwrap()
    }

    fn actman_state<A: actman::Actor + ?Sized>()
    -> (State<A>, mpsc::Sender<A::Message>, mpsc::Sender<Control>) {
        let (message_sender, message_receiver) = mpsc::channel(10);
        let (control_sender, control_receiver) = mpsc::channel(1);
        (
            State {
                message_receiver,
                control_receiver,
            },
            message_sender,
            control_sender,
        )
    }
}
