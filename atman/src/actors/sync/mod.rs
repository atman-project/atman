mod config;
mod handle;
pub mod message;

use std::{
    collections::{HashMap, HashSet},
    fmt::Debug,
    io,
    path::PathBuf,
};

pub use config::Config;
pub use handle::SyncHandle;
use syncman::{Syncman, automerge::AutomergeSyncman};
use tokio::sync::oneshot;
use tracing::{error, info, warn};

use crate::{
    doc::{self, DocId, DocSpace, Document, DocumentResolver},
    sync::message::Message,
    sync_message::{GetMessage, InitiateSyncMessage, ListInsertMessage, UpdateMessage},
};

pub struct Actor {
    config: Config,
    syncmans: HashMap<(DocSpace, DocId), AutomergeSyncman>,
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

        let mut syncmans = HashMap::new();
        for (doc_space, doc_id, path) in config.syncman_paths()?.into_iter() {
            let syncman = load_syncman(&path)?;
            syncmans.insert((doc_space, doc_id), syncman);
        }

        Ok(Self {
            config,
            syncmans,
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
            Message::ListDocuments { reply_sender } => self.handle_list_documents(reply_sender),
            Message::InitiateSync { msg, reply_sender } => {
                self.handle_initiate_sync_message(msg, reply_sender)
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
        self.prepare_initial_syncman(&doc_space, &doc_id)?;
        let syncman = self
            .syncmans
            .get_mut(&(doc_space.clone(), doc_id.clone()))
            .expect("syncman must exist");
        syncman.update(&doc);
        info!("Document updated in syncman");
        save_syncman(syncman, &self.config.syncman_path(&doc_space, &doc_id))
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
            collection_doc_id,
            doc_id,
            property,
            data,
            index,
        }: ListInsertMessage,
    ) -> Result<(), Error> {
        let doc = self.doc_resolver.deserialize(&doc_space, &doc_id, &data)?;
        let syncman = self
            .syncmans
            .get_mut(&(doc_space.clone(), collection_doc_id.clone()))
            .ok_or(Error::DocumentNotFound {
                doc_space: doc_space.clone(),
                doc_id: collection_doc_id.clone(),
            })?;
        let obj_id = syncman.get_object_id(syncman.root(), property.clone());
        syncman.insert(obj_id, index, &doc);
        info!("Inserted into a list. prop:{property}, index:{index}");
        save_syncman(
            syncman,
            &self.config.syncman_path(&doc_space, &collection_doc_id),
        )
    }

    fn handle_get_message(
        &mut self,
        msg: GetMessage,
        reply_sender: oneshot::Sender<Result<Document, Error>>,
    ) {
        let _ = reply_sender
            .send(
                self.handle_get_message_inner(msg)
                    .inspect_err(|e| error!("Failed to handle get message: {e:?}")),
            )
            .inspect_err(|_| error!("Failed to send reply"));
    }

    fn handle_get_message_inner(
        &mut self,
        GetMessage { doc_space, doc_id }: GetMessage,
    ) -> Result<Document, Error> {
        let _ = self.prepare_initial_syncman(&doc_space, &doc_id);
        let syncman = self
            .syncmans
            .get(&(doc_space.clone(), doc_id.clone()))
            .ok_or(Error::DocumentNotFound {
                doc_space: doc_space.clone(),
                doc_id: doc_id.clone(),
            })?;
        Ok(self.doc_resolver.hydrate(&doc_space, &doc_id, syncman)?)
    }

    fn handle_list_documents(&self, reply_sender: oneshot::Sender<HashSet<(DocSpace, DocId)>>) {
        let docs = self.syncmans.keys().cloned().collect();
        let _ = reply_sender
            .send(docs)
            .inspect_err(|_| error!("Failed to send reply"));
    }

    fn handle_initiate_sync_message(
        &mut self,
        msg: InitiateSyncMessage,
        reply_sender: oneshot::Sender<Result<SyncHandle, Error>>,
    ) {
        let _ = reply_sender
            .send(
                self.handle_initiate_sync_message_inner(msg)
                    .inspect_err(|e| error!("Failed to handle initiate sync message: {e:?}")),
            )
            .inspect_err(|_| error!("Failed to send reply"));
    }

    fn handle_initiate_sync_message_inner(
        &mut self,
        InitiateSyncMessage { doc_space, doc_id }: InitiateSyncMessage,
    ) -> Result<SyncHandle, Error> {
        self.prepare_initial_syncman(&doc_space, &doc_id)?;
        let syncman = self
            .syncmans
            .get_mut(&(doc_space.clone(), doc_id.clone()))
            .expect("syncman must exist");
        Ok(SyncHandle::new(syncman.initiate_sync(), doc_space, doc_id))
    }

    fn handle_apply_sync_message(
        &mut self,
        data: Vec<u8>,
        handle: SyncHandle,
        reply_sender: oneshot::Sender<Result<SyncHandle, Error>>,
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
        mut handle: SyncHandle,
    ) -> Result<SyncHandle, Error> {
        let (doc_space, doc_id) = handle.doc_space_and_id();
        let syncman = self
            .syncmans
            .get_mut(&(doc_space.clone(), doc_id.clone()))
            .ok_or(Error::DocumentNotFound {
                doc_space: doc_space.clone(),
                doc_id: doc_id.clone(),
            })?;
        syncman.apply_sync(handle.syncman_handle_mut(), &data);
        save_syncman(syncman, &self.config.syncman_path(&doc_space, &doc_id))?;
        Ok(handle)
    }

    fn prepare_initial_syncman(
        &mut self,
        doc_space: &DocSpace,
        doc_id: &DocId,
    ) -> Result<(), Error> {
        if self
            .syncmans
            .contains_key(&(doc_space.clone(), doc_id.clone()))
        {
            return Ok(());
        }

        match self.initial_syncman(doc_space, doc_id) {
            Some(syncman) => {
                self.syncmans
                    .insert((doc_space.clone(), doc_id.clone()), syncman);
                Ok(())
            }
            None => Err(Error::InitialDocumentNotRegistered {
                doc_space: doc_space.clone(),
                doc_id: doc_id.clone(),
            }),
        }
    }

    fn initial_syncman(&self, doc_space: &DocSpace, doc_id: &DocId) -> Option<AutomergeSyncman> {
        self.doc_resolver
            .initial_document(doc_space, doc_id)
            .map(AutomergeSyncman::load)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("IO error: {message}: {cause}")]
    IO { message: String, cause: io::Error },
    #[error("Document error: {0}")]
    Document(#[from] doc::Error),
    #[error("Document not found: {doc_space:?}, {doc_id:?}")]
    DocumentNotFound { doc_space: DocSpace, doc_id: DocId },
    #[error("Initial document not registered: {doc_space:?}, {doc_id:?}")]
    InitialDocumentNotRegistered { doc_space: DocSpace, doc_id: DocId },
}

fn save_syncman(syncman: &mut AutomergeSyncman, path: &PathBuf) -> Result<(), Error> {
    let data = syncman.save();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::IO {
            message: format!("Failed to create directory for syncman file. parent: {parent:?}"),
            cause: e,
        })?;
    }
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
    use actman::{Actor as _, Control, State};
    use syncman::SyncHandle;
    use tempfile::TempDir;
    use tokio::sync::mpsc;
    use uuid::Uuid;

    use super::*;
    use crate::doc::{
        Document,
        aviation::{self, flight::Flight, flights::Flights},
    };

    #[test]
    fn test_new() {
        let config = sync_config();
        let _actor = Actor::new(config.clone()).unwrap();
        // Check if the syncman dir has been created.
        assert!(config.syncman_dir.exists());
    }

    #[test_log::test(tokio::test)]
    async fn initiate_sync_and_generate_msg() {
        let actor = Actor::new(sync_config()).unwrap();
        let (state, message_sender, _control_sender) = actman_state::<Actor>();
        tokio::spawn(async move {
            actor.run(state).await;
        });

        let (msg, reply_receiver) = InitiateSyncMessage {
            doc_space: aviation::DOC_SPACE.into(),
            doc_id: aviation::flights::DOC_ID.into(),
        }
        .into();
        message_sender.send(msg).await.unwrap();
        let mut handle = reply_receiver.await.unwrap().unwrap();

        let msg = handle
            .syncman_handle_mut()
            .generate_message()
            .expect("sync message must be generated even if the document is empty");
        assert!(!msg.is_empty())
    }

    #[test_log::test(tokio::test)]
    async fn update() {
        let actor = Actor::new(sync_config()).unwrap();
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
            collection_doc_id: aviation::flights::DOC_ID.into(),
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

    #[test_log::test(tokio::test)]
    async fn initialize_and_get() {
        let actor = Actor::new(sync_config()).unwrap();
        let (state, message_sender, _control_sender) = actman_state::<Actor>();
        tokio::spawn(async move {
            actor.run(state).await;
        });

        // Expect that Flights doc is initialized and returned.
        let (msg, reply_receiver) = GetMessage {
            doc_space: aviation::DOC_SPACE.into(),
            doc_id: aviation::flights::DOC_ID.into(),
        }
        .into();
        message_sender.send(msg).await.unwrap();
        let doc = reply_receiver.await.unwrap().unwrap();
        assert_eq!(doc, Document::Flights(Flights { flights: vec![] }));

        // Add a flight to the Flights doc.
        let flight = Flight {
            id: Uuid::new_v4(),
            departure_airport: "ICN".into(),
            arrival_airport: "SEA".into(),
            departure_local_time: "2025-08-03T05:45:10Z".into(),
            arrival_local_time: "2025-08-03T05:45:10Z".into(),
            airline: "Korean Air".into(),
            aircraft: "A350-900".into(),
            flight_number: "KE1234".into(),
            booking_reference: "ABCDEF".into(),
        };
        let (msg, reply_receiver) = ListInsertMessage {
            doc_space: aviation::DOC_SPACE.into(),
            collection_doc_id: aviation::flights::DOC_ID.into(),
            doc_id: aviation::flight::DOC_ID.into(),
            property: "flights".into(),
            index: 0,
            data: Document::Flight(flight.clone()).serialize().unwrap(),
        }
        .into();
        message_sender.send(msg).await.unwrap();
        reply_receiver.await.unwrap().unwrap();

        // Expect that the updated doc is returned.
        let (msg, reply_receiver) = GetMessage {
            doc_space: aviation::DOC_SPACE.into(),
            doc_id: aviation::flights::DOC_ID.into(),
        }
        .into();
        message_sender.send(msg).await.unwrap();
        let doc = reply_receiver.await.unwrap().unwrap();
        assert_eq!(
            doc,
            Document::Flights(Flights {
                flights: vec![flight]
            })
        );

        // Error::DocumentNotFound is returned for Flight doc.
        let (msg, reply_receiver) = GetMessage {
            doc_space: aviation::DOC_SPACE.into(),
            doc_id: aviation::flight::DOC_ID.into(),
        }
        .into();
        message_sender.send(msg).await.unwrap();
        let doc = reply_receiver.await.unwrap().unwrap_err();
        assert!(matches!(doc, Error::DocumentNotFound { .. }));
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

    fn sync_config() -> Config {
        Config {
            syncman_dir: TempDir::new().unwrap().path().to_owned(),
        }
    }
}
