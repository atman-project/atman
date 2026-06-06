//! In-memory blob send/receive built on top of `iroh-blobs`.
//!
//! The network actor owns one [`Blobs`]. On construction it returns
//! both itself and a [`iroh_blobs::BlobsProtocol`] that must be registered
//! on the same [`iroh::protocol::Router`] as the rest of atman's protocols,
//! so the standard iroh-blobs pull flow works.
//!
//! Usage:
//!
//! - Sender side: [`Blobs::add_files`] imports one or more files into the local
//!   store, and returns a [`BlobTicket`]. For a single file, the resulting
//!   ticket points at the raw blob. For multiple files, it points at a
//!   [`Collection`].
//! - Receiver side: [`Blobs::download`] takes a ticket, dials the sender, pulls
//!   every needed blob into our store, then exports each one into the save
//!   directory.

pub mod ticket;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use iroh::Endpoint;
use iroh_blobs::{
    BlobFormat, BlobsProtocol, Hash,
    format::collection::Collection,
    provider::events::{
        ConnectMode, EventMask, EventSender, ProviderMessage, RequestMode, RequestUpdate,
    },
    store::mem::MemStore,
    ticket::BlobTicket as IrohBlobTicket,
};
use ticket::BlobTicket;
use tokio::sync::Mutex;
use tracing::debug;

#[derive(Debug)]
pub struct Blobs {
    endpoint: Endpoint,
    store: MemStore,
    /// Per-ticket receiver counts, keyed by the ticket's hash (Raw blob
    /// hash or HashSeq root). Each successful GET request bumps the
    /// counter for the hash the receiver asked for. Shared with the
    /// background event-handler task, which is why it's `Arc<Mutex<…>>`.
    // TODO: bound the map size — every ticket ever issued lives here.
    transfer_counts: Arc<Mutex<HashMap<Hash, u64>>>,
}

impl Blobs {
    /// Create [`Blobs`] and its corresponding protocol handler. The caller
    /// must register the returned protocol on the same
    /// [`iroh::protocol::Router`] using [`iroh_blobs::ALPN`].
    pub fn new(endpoint: Endpoint) -> (Self, BlobsProtocol) {
        let store = MemStore::new();
        let transfer_counts = Arc::new(Mutex::new(HashMap::new()));
        let event_sender = spawn_transfer_counter(transfer_counts.clone());
        let protocol = BlobsProtocol::new(&store, Some(event_sender));
        let manager = Self {
            endpoint,
            store,
            transfer_counts,
        };
        (manager, protocol)
    }

    /// How many receivers have fully pulled the ticket's hash.
    pub async fn transfer_count(&self, hash: Hash) -> Option<u64> {
        self.transfer_counts.lock().await.get(&hash).copied()
    }

    /// Import files into the local blob store and return a ticket the
    /// caller can share.
    ///
    /// A single path produces a raw-format ticket.
    /// 2+ pathes produce a `HashSeq` ticket pointing at a [`Collection`].
    /// The two formats are kept distinct so single-file shares stay on
    /// the smaller raw wire-format.
    pub async fn add_files(&self, paths: Vec<PathBuf>) -> Result<BlobTicket, Error> {
        if paths.is_empty() {
            return Err(Error::Empty);
        }

        if paths.len() == 1 {
            self.add_single(paths.into_iter().next().expect("path must exist"))
                .await
        } else {
            self.add_multiple(paths).await
        }
    }

    async fn add_single(&self, path: PathBuf) -> Result<BlobTicket, Error> {
        let abs = std::path::absolute(&path).map_err(Error::Io)?;
        let filename = abs
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or(Error::InvalidFilename)?
            .to_string();

        let tag = self
            .store
            .blobs()
            .add_path(abs.clone())
            .await
            .map_err(|e| Error::Blob(e.to_string()))?;
        debug!(hash = %tag.hash, ?abs, "imported single file");

        let iroh_ticket = IrohBlobTicket::new(self.endpoint.addr(), tag.hash, tag.format);
        Ok(BlobTicket::new(iroh_ticket, vec![filename]))
    }

    /// Import every child, then store a [`Collection`] that lists them by
    /// (filename, hash).
    async fn add_multiple(&self, paths: Vec<PathBuf>) -> Result<BlobTicket, Error> {
        let mut filenames = Vec::with_capacity(paths.len());
        let mut entries: Vec<(String, iroh_blobs::Hash)> = Vec::with_capacity(paths.len());
        for path in paths {
            let abs = std::path::absolute(&path).map_err(Error::Io)?;
            let filename = abs
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or(Error::InvalidFilename)?
                .to_string();
            let tag = self
                .store
                .blobs()
                .add_path(abs.clone())
                .await
                .map_err(|e| Error::Blob(e.to_string()))?;
            debug!(hash = %tag.hash, ?abs, "imported a file (a collection child)");
            entries.push((filename.clone(), tag.hash));
            filenames.push(filename);
        }

        let collection: Collection = entries.into_iter().collect();
        let root_tag = collection
            .store(&self.store)
            .await
            .map_err(|e| Error::Blob(e.to_string()))?;
        let root_hash = root_tag.hash();
        debug!(hash = %root_hash, count = filenames.len(), "stored a collection root");

        // Embed the live `EndpointAddr` (relay URL + direct UDP addrs)
        // in the ticket, not just the `EndpointId` — otherwise the receiver has
        // nothing to dial with and has to fall back to discovery.
        let ticket = IrohBlobTicket::new(self.endpoint.addr(), root_hash, BlobFormat::HashSeq);
        Ok(BlobTicket::new(ticket, filenames))
    }

    /// Pull the blob(s) described by [`BlobTicket`] from the sender
    /// into the local store, then export every contained file into
    /// `save_dir`. Returns the on-disk path of each saved file.
    pub async fn download(
        &self,
        ticket: BlobTicket,
        save_dir: PathBuf,
    ) -> Result<Vec<PathBuf>, Error> {
        tokio::fs::create_dir_all(&save_dir)
            .await
            .map_err(Error::Io)?;

        // Dial the sender directly using the full `EndpointAddr` carried
        // in the ticket.
        let conn = self
            .endpoint
            .connect(ticket.inner.addr().clone(), iroh_blobs::ALPN)
            .await
            .map_err(|e| Error::Blob(e.to_string()))?;

        // Fetch blobs from the sender
        self.store
            .remote()
            .fetch(conn, ticket.inner.hash_and_format())
            .complete()
            .await
            .map_err(|e| Error::Blob(e.to_string()))?;

        match ticket.inner.format() {
            BlobFormat::Raw => self
                .export_single(&ticket, &save_dir)
                .await
                .map(|p| vec![p]),
            BlobFormat::HashSeq => self.export_collection(&ticket, &save_dir).await,
        }
    }

    /// Exports a single raw blob fetched from the sender into `save_dir`.
    /// Returns the on-disk path of the saved file.
    async fn export_single(&self, ticket: &BlobTicket, save_dir: &Path) -> Result<PathBuf, Error> {
        let filename = ticket
            .filenames
            .first()
            .cloned()
            .unwrap_or_else(|| "unnamed".to_string());
        let path = unique_path(save_dir, &filename);
        self.store
            .blobs()
            .export(ticket.inner.hash(), &path)
            .await
            .map_err(|e| Error::Blob(e.to_string()))?;
        Ok(path)
    }

    async fn export_collection(
        &self,
        ticket: &BlobTicket,
        save_dir: &Path,
    ) -> Result<Vec<PathBuf>, Error> {
        // Read the Collection manifest from our local store (it just got
        // downloaded by the call above). We don't trust the ticket's
        // filenames blindly — Collection is the canonical source of truth
        // for what (name, hash) pairs are inside the bundle.
        // MemStore derefs to api::Store which implements SimpleStore — the
        // trait Collection::load wants. Coerce explicitly so the trait
        // bound resolves.
        let collection = Collection::load(ticket.inner.hash(), self.store.as_ref())
            .await
            .map_err(|e| Error::Blob(e.to_string()))?;

        let mut saved = Vec::with_capacity(collection.len());
        for (name, hash) in collection.iter() {
            let path = unique_path(save_dir, name);
            self.store
                .blobs()
                .export(*hash, &path)
                .await
                .map_err(|e| Error::Blob(e.to_string()))?;
            saved.push(path);
        }
        Ok(saved)
    }
}

/// Wire up the iroh-blobs serve-side event stream and return an
/// [`EventSender`] suitable to hand to [`BlobsProtocol::new`].
fn spawn_transfer_counter(counts: Arc<Mutex<HashMap<Hash, u64>>>) -> EventSender {
    let mask = EventMask {
        connected: ConnectMode::None,
        get: RequestMode::NotifyLog,
        get_many: RequestMode::NotifyLog,
        ..EventMask::DEFAULT
    };
    let (sender, mut receiver) = EventSender::channel(32, mask);

    tokio::spawn(async move {
        while let Some(msg) = receiver.recv().await {
            if let ProviderMessage::GetRequestReceivedNotify(req) = msg {
                let hash = req.inner.request.hash;
                tokio::spawn(handle_request_updates(req.rx, counts.clone(), hash));
            }
        }
    });

    sender
}

/// Drains one request's update stream and bumps `transfer_counts[hash]`
/// only if the stream's final event was `Completed` — i.e. the receiver
/// got through the last blob cleanly. `Aborted` and mid-blob drops
/// don't count.
async fn handle_request_updates(
    mut updates: irpc::channel::mpsc::Receiver<RequestUpdate>,
    transfer_counts: Arc<Mutex<HashMap<Hash, u64>>>,
    hash: Hash,
) {
    let mut last_was_completed = false;
    while let Ok(Some(update)) = updates.recv().await {
        last_was_completed = matches!(update, RequestUpdate::Completed(_));
    }
    if last_was_completed {
        *transfer_counts.lock().await.entry(hash).or_insert(0) += 1;
    }
}

/// Builds an unique file path in `dir` based on `name` by adding a suffix if
/// there is already a file with the same name.
fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    for i in 1..10_000 {
        let candidate = dir.join(format!("{stem} ({i}).{ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{stem}-{}.{ext}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use iroh::{Endpoint, protocol::Router};
    use tokio::time::timeout;

    use super::*;

    #[test_log::test(tokio::test(flavor = "multi_thread"))]
    async fn single_file() {
        const TEST_DEADLINE: Duration = Duration::from_secs(30);
        let body = b"hello atman blobs!".as_slice();

        let (sender_endpoint, sender, _sender_router) = build_endpoint().await;
        timeout(TEST_DEADLINE, sender_endpoint.online())
            .await
            .unwrap();

        let tmp_send_dir = tempfile::tempdir().unwrap();
        let src_file = tmp_send_dir.path().join("greeting.txt");
        tokio::fs::write(&src_file, body).await.unwrap();
        let ticket = sender.add_files(vec![src_file]).await.unwrap();
        assert_eq!(ticket.filenames, vec!["greeting.txt"]);
        assert_eq!(ticket.inner.format(), BlobFormat::Raw);

        let (recv_endpoint, receiver, _recv_router) = build_endpoint().await;
        timeout(TEST_DEADLINE, recv_endpoint.online())
            .await
            .unwrap();

        let parsed: BlobTicket = ticket.to_string().parse().unwrap();
        let save_dir = tempfile::tempdir().unwrap();
        let saved = timeout(
            TEST_DEADLINE,
            receiver.download(parsed, save_dir.path().to_path_buf()),
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].file_name().unwrap(), "greeting.txt");
        let received = tokio::fs::read(&saved[0]).await.unwrap();
        assert_eq!(received, body);

        // Wait briefly for the serve-side event task to observe the
        // request stream closing — it runs on a separate tokio task so a
        // small grace period is needed before reading the counter.
        let hash = ticket.inner.hash();
        for _ in 0..50 {
            if sender.transfer_count(hash).await.unwrap() >= 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(sender.transfer_count(hash).await.unwrap(), 1);
    }

    #[test_log::test(tokio::test(flavor = "multi_thread"))]
    async fn multiple_files() {
        const TEST_DEADLINE: Duration = Duration::from_secs(30);
        let bodies: [(&str, &[u8]); 3] = [
            ("first.txt", b"first one"),
            ("second.bin", b"\x00\x01\x02 second"),
            ("third.log", b"third\nwith newlines\n"),
        ];

        let (sender_endpoint, sender, _sender_router) = build_endpoint().await;
        timeout(TEST_DEADLINE, sender_endpoint.online())
            .await
            .unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let mut paths = Vec::new();
        for (name, body) in &bodies {
            let p = tmp.path().join(name);
            tokio::fs::write(&p, body).await.unwrap();
            paths.push(p);
        }
        let ticket = sender.add_files(paths).await.unwrap();
        assert_eq!(ticket.filenames.len(), 3);
        assert_eq!(ticket.inner.format(), BlobFormat::HashSeq);

        let (recv_endpoint, receiver, _recv_router) = build_endpoint().await;
        timeout(TEST_DEADLINE, recv_endpoint.online())
            .await
            .unwrap();

        let parsed: BlobTicket = ticket.to_string().parse().unwrap();
        let save_dir = tempfile::tempdir().unwrap();
        let saved = timeout(
            TEST_DEADLINE,
            receiver.download(parsed, save_dir.path().to_path_buf()),
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(saved.len(), 3);
        for ((name, body), got) in bodies.iter().zip(&saved) {
            assert_eq!(got.file_name().unwrap().to_str().unwrap(), *name);
            let received = tokio::fs::read(got).await.unwrap();
            assert_eq!(&received[..], *body);
        }
    }

    async fn build_endpoint() -> (Endpoint, Blobs, Router) {
        let endpoint = Endpoint::builder().bind().await.unwrap();
        let (manager, proto) = Blobs::new(endpoint.clone());
        let router = Router::builder(endpoint.clone())
            .accept(iroh_blobs::ALPN, proto)
            .spawn();
        (endpoint, manager, router)
    }
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("at least one path is required")]
    Empty,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("filename is not valid utf-8")]
    InvalidFilename,
    #[error("iroh-blobs error: {0}")]
    Blob(String),
}
