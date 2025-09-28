use std::fmt::{self, Debug, Formatter};

use serde::{Deserialize, Serialize};
use syncman::automerge::AutomergeSyncHandle;
use tokio::sync::oneshot;

use crate::doc::{DocId, DocSpace, Document};

#[expect(
    clippy::large_enum_variant,
    reason = "Make AutomergeSyncHandle generic"
)]
pub enum Message {
    Update(UpdateMessage),
    ListInsert(ListInsertMessage),
    Get {
        msg: GetMessage,
        reply_sender: oneshot::Sender<Document>,
    },
    InitiateSync {
        reply_sender: oneshot::Sender<AutomergeSyncHandle>,
    },
    ApplySync {
        data: Vec<u8>,
        handle: AutomergeSyncHandle,
        reply_sender: oneshot::Sender<AutomergeSyncHandle>,
    },
}

impl Debug for Message {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Update(msg) => f.debug_tuple("Update").field(msg).finish(),
            Self::ListInsert(msg) => f.debug_tuple("ListInsert").field(msg).finish(),
            Self::Get { msg, .. } => f.debug_tuple("Get").field(msg).finish(),
            Self::InitiateSync { .. } => f.debug_tuple("InitiateSync").finish(),
            Self::ApplySync { .. } => f.debug_tuple("ApplySync").finish(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateMessage {
    pub doc_space: DocSpace,
    pub doc_id: DocId,
    pub data: SerializedModel,
}

impl From<UpdateMessage> for Message {
    fn from(msg: UpdateMessage) -> Self {
        Self::Update(msg)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListInsertMessage {
    pub doc_space: DocSpace,
    pub doc_id: DocId,
    pub property: String,
    pub data: SerializedModel,
    pub index: usize,
}

impl From<ListInsertMessage> for Message {
    fn from(msg: ListInsertMessage) -> Self {
        Self::ListInsert(msg)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetMessage {
    pub doc_space: DocSpace,
    pub doc_id: DocId,
}

impl From<GetMessage> for (Message, oneshot::Receiver<Document>) {
    fn from(msg: GetMessage) -> Self {
        let (reply_sender, reply_receiver) = oneshot::channel();
        (Message::Get { msg, reply_sender }, reply_receiver)
    }
}

type SerializedModel = Vec<u8>;
