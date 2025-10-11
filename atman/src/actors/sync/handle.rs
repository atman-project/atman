use syncman::automerge::AutomergeSyncHandle;

use crate::doc::{DocId, DocSpace};

pub struct SyncHandle {
    handle: AutomergeSyncHandle,
    doc_space: DocSpace,
    doc_id: DocId,
}

impl SyncHandle {
    pub fn new(handle: AutomergeSyncHandle, doc_space: DocSpace, doc_id: DocId) -> Self {
        Self {
            handle,
            doc_space,
            doc_id,
        }
    }

    pub fn syncman_handle_mut(&mut self) -> &mut AutomergeSyncHandle {
        &mut self.handle
    }

    pub fn doc_space_and_id(&self) -> (DocSpace, DocId) {
        (self.doc_space.clone(), self.doc_id.clone())
    }
}
