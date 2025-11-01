use std::{
    fmt::{self, Debug, Formatter},
    io,
};

use iroh::{
    EndpointId,
    endpoint::Connection,
    protocol::{AcceptError, ProtocolHandler, Router},
};
use serde::{Deserialize, Serialize};
use syncman::{SyncHandle, automerge::AutomergeSyncHandle};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{mpsc, oneshot},
};
use tracing::{error, info};

use crate::{
    actors::network::{
        Error,
        protocols::{close_conn, connect, wait_conn_closed},
    },
    doc::{DocId, DocSpace},
    sync_message::InitiateSyncMessage,
};

#[derive(Clone)]
pub struct Protocol {
    event_sender: mpsc::Sender<Event>,
    sync_actor_handle: actman::Handle<crate::sync::Actor>,
}

impl Debug for Protocol {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Protocol").finish()
    }
}

impl ProtocolHandler for Protocol {
    /// The `accept` method is called for each incoming connection for our ALPN.
    ///
    /// The returned future runs on a newly spawned tokio task, so it can run as
    /// long as the connection lasts.
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        self.clone().handle_connection(connection).await
    }
}

impl Protocol {
    pub const ALPN: &[u8] = b"atman/sync/0";

    pub fn new(
        event_sender: mpsc::Sender<Event>,
        sync_actor_handle: actman::Handle<crate::sync::Actor>,
    ) -> Self {
        Self {
            event_sender,
            sync_actor_handle,
        }
    }
}

impl Protocol {
    async fn handle_connection(self, connection: Connection) -> Result<(), AcceptError> {
        // Wait for the connection to be fully established.
        let node_id = connection.remote_id()?;
        if let Err(e) = self.event_sender.send(Event::Accepted { node_id }).await {
            error!("Failed to send sync event to the channel: {e:?}");
        }
        let res = self.handle_connection_0(&connection).await;
        let error = res.as_ref().err().map(|err| err.to_string());
        if let Err(e) = self
            .event_sender
            .send(Event::Closed { node_id, error })
            .await
        {
            error!("Failed to send sync event to the channel: {e:?}");
        }
        res
    }

    async fn handle_connection_0(&self, connection: &Connection) -> Result<(), AcceptError> {
        // We can get the remote's node id from the connection.
        let node_id = connection.remote_id()?;
        info!("Accepted connection from {node_id}");

        // Our protocol is a simple request-response protocol, so we expect the
        // connecting peer to open a single bi-directional stream.
        let (mut send, mut recv) = connection.accept_bi().await?;
        info!("Accepted stream");

        if let Err(e) = listen_and_perform_sync(&self.sync_actor_handle, &mut send, &mut recv).await
        {
            error!("Failed to listen and perform sync: {e}");
            return Err(AcceptError::from_err(e));
        }

        wait_conn_closed(connection, &mut send).await;
        Ok(())
    }

    pub async fn connect_and_spawn(
        node_id: EndpointId,
        doc_space: DocSpace,
        doc_id: DocId,
        router: &Router,
        sync_actor_handle: &actman::Handle<crate::sync::Actor>,
        reply_sender: oneshot::Sender<Result<(), Error>>,
    ) {
        let (conn, mut send_stream, mut recv_stream) =
            match connect(node_id, router, Self::ALPN).await {
                Ok(streams) => streams,
                Err(e) => {
                    let _ = reply_sender
                        .send(Err(e))
                        .inspect_err(|_| error!("failed to send reply"));
                    return;
                }
            };
        info!("Stream opened");

        let sync_actor_handle = sync_actor_handle.clone();
        tokio::spawn(async move {
            let result = trigger_and_perform_sync(
                &sync_actor_handle,
                doc_space,
                doc_id,
                &mut send_stream,
                &mut recv_stream,
            )
            .await
            .inspect_err(|e| error!("Failed to perform sync: {e:?}"));
            close_conn(&conn, &mut send_stream, result.is_err()).await;
            let _ = reply_sender
                .send(result)
                .inspect_err(|_| error!("failed to send reply"));
        });
    }
}

async fn trigger_and_perform_sync<SendStream, RecvStream>(
    sync_actor_handle: &actman::Handle<crate::sync::Actor>,
    doc_space: DocSpace,
    doc_id: DocId,
    send_stream: &mut SendStream,
    recv_stream: &mut RecvStream,
) -> Result<(), Error>
where
    SendStream: AsyncWriteExt + Unpin,
    RecvStream: AsyncReadExt + Unpin,
{
    send_doc_space_and_id(&doc_space, &doc_id, send_stream).await?;
    perform_sync(
        sync_actor_handle,
        doc_space,
        doc_id,
        send_stream,
        recv_stream,
        true,
    )
    .await
}

async fn listen_and_perform_sync<SendStream, RecvStream>(
    sync_actor_handle: &actman::Handle<crate::sync::Actor>,
    send_stream: &mut SendStream,
    recv_stream: &mut RecvStream,
) -> Result<(), Error>
where
    SendStream: AsyncWriteExt + Unpin,
    RecvStream: AsyncReadExt + Unpin,
{
    let (doc_space, doc_id) = read_doc_space_and_id(recv_stream).await?;
    perform_sync(
        sync_actor_handle,
        doc_space,
        doc_id,
        send_stream,
        recv_stream,
        false,
    )
    .await
}

async fn perform_sync<SendStream, RecvStream>(
    sync_actor_handle: &actman::Handle<crate::sync::Actor>,
    doc_space: DocSpace,
    doc_id: DocId,
    send_stream: &mut SendStream,
    recv_stream: &mut RecvStream,
    as_trigger: bool,
) -> Result<(), Error>
where
    SendStream: AsyncWriteExt + Unpin,
    RecvStream: AsyncReadExt + Unpin,
{
    let (msg, reply_receiver) = InitiateSyncMessage { doc_space, doc_id }.into();
    sync_actor_handle.send(msg).await;
    let mut sync_handle = reply_receiver
        .await
        .expect("Sync actor must be available")?;

    if as_trigger
        && !generate_and_send_sync_msg(sync_handle.syncman_handle_mut(), send_stream).await?
    {
        return Err(Error::InitialSyncMessageNotGenerated);
    }

    loop {
        match read_sync_msg(recv_stream).await? {
            Some(msg) => {
                info!("Received sync message of length {}", msg.len());
                let (reply_sender, reply_receiver) = oneshot::channel();
                sync_actor_handle
                    .send(crate::sync::message::Message::ApplySync {
                        data: msg,
                        handle: sync_handle,
                        reply_sender,
                    })
                    .await;
                sync_handle = reply_receiver
                    .await
                    .expect("Sync actor must be available")?;
            }
            None => {
                info!("No more sync messages from peer. Syncing has been done.");
                return Ok(());
            }
        }

        if !generate_and_send_sync_msg(sync_handle.syncman_handle_mut(), send_stream).await? {
            return Ok(());
        }
    }
}

/// Generate a sync message, send it to the peer, and return `true`.
/// If there is no more message to send, send a finish message and return
/// `false`.
async fn generate_and_send_sync_msg<Stream>(
    sync_handle: &mut AutomergeSyncHandle,
    send_stream: &mut Stream,
) -> Result<bool, Error>
where
    Stream: AsyncWriteExt + Unpin,
{
    if let Some(msg) = sync_handle.generate_message() {
        send_sync_msg(&msg, send_stream).await?;
        Ok(true)
    } else {
        info!("Nothing to sync. Syncing has been done.");
        send_finish(send_stream).await?;
        Ok(false)
    }
}

async fn send_doc_space_and_id<Stream>(
    doc_space: &DocSpace,
    doc_id: &DocId,
    stream: &mut Stream,
) -> Result<(), Error>
where
    Stream: AsyncWriteExt + Unpin,
{
    send_data(doc_space.as_bytes(), stream).await?;
    send_data(doc_id.as_bytes(), stream).await?;
    Ok(())
}

async fn read_doc_space_and_id<Stream>(stream: &mut Stream) -> Result<(DocSpace, DocId), Error>
where
    Stream: AsyncReadExt + Unpin,
{
    let doc_space = read_data(stream)
        .await?
        .ok_or(Error::UnexpectedFinishMessage)?
        .as_slice()
        .try_into()?;
    let doc_id = read_data(stream)
        .await?
        .ok_or(Error::UnexpectedFinishMessage)?
        .as_slice()
        .try_into()?;
    Ok((doc_space, doc_id))
}

async fn send_sync_msg<Stream>(msg: &[u8], stream: &mut Stream) -> Result<(), Error>
where
    Stream: AsyncWriteExt + Unpin,
{
    if msg.is_empty() {
        return Ok(send_finish(stream).await?);
    }

    assert!(!msg.is_empty());
    info!("Sending sync message to peer: len:{}", msg.len());
    Ok(send_data(msg, stream).await?)
}

async fn read_sync_msg<Stream>(stream: &mut Stream) -> Result<Option<Vec<u8>>, Error>
where
    Stream: AsyncReadExt + Unpin,
{
    info!("Reading sync response from peer");
    Ok(read_data(stream).await?)
}

async fn send_data<Stream>(data: &[u8], stream: &mut Stream) -> io::Result<()>
where
    Stream: AsyncWriteExt + Unpin,
{
    let len: u64 = data.len().try_into().expect("data len must be u64");
    stream.write_u64(len).await?;
    stream.write_all(data).await?;
    Ok(())
}

async fn send_finish<Stream>(stream: &mut Stream) -> io::Result<()>
where
    Stream: AsyncWriteExt + Unpin,
{
    stream.write_u64(0u64).await
}

async fn read_data<Stream>(stream: &mut Stream) -> io::Result<Option<Vec<u8>>>
where
    Stream: AsyncReadExt + Unpin,
{
    let len: usize = stream
        .read_u64()
        .await?
        .try_into()
        .expect("data len must be usize");
    if len == 0 {
        return Ok(None);
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    Ok(Some(buf))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Event {
    Accepted {
        node_id: EndpointId,
    },
    Closed {
        node_id: EndpointId,
        error: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::{actors::sync, doc};

    #[test_log::test(tokio::test)]
    async fn test_sync() {
        let sync_actor_0 = sync::Actor::new(sync_config()).unwrap();
        let sync_actor_1 = sync::Actor::new(sync_config()).unwrap();

        let mut runner = actman::Runner::new();
        let sync_handle_0 = runner.run(sync_actor_0);
        let sync_handle_1 = runner.run(sync_actor_1);

        let (mut listener_recv, mut trigger_send) = tokio::io::simplex(10);
        let (mut trigger_recv, mut listener_send) = tokio::io::simplex(10);

        // Run listener side
        let join_listener = tokio::spawn(async move {
            listen_and_perform_sync(&sync_handle_1, &mut listener_send, &mut listener_recv)
                .await
                .expect("sync must succeeded");
        });

        // Prepare a doc to sync
        let doc_space: DocSpace = doc::protocol::DOC_SPACE.into();
        let doc_id: DocId = doc::protocol::nodes::DOC_ID.into();
        let doc = doc::Document::Nodes(doc::protocol::nodes::Nodes { nodes: Vec::new() });
        let (msg, reply_receiver) = sync::message::UpdateMessage {
            doc_space: doc_space.clone(),
            doc_id: doc_id.clone(),
            data: doc.serialize().unwrap(),
        }
        .into();
        sync_handle_0.send(msg).await;
        reply_receiver.await.unwrap().unwrap();

        // Run trigger side
        trigger_and_perform_sync(
            &sync_handle_0,
            doc_space,
            doc_id,
            &mut trigger_send,
            &mut trigger_recv,
        )
        .await
        .unwrap();
        // Wait for listener to finish without error.
        let () = join_listener.await.unwrap();

        runner.shutdown().await;
    }

    fn sync_config() -> sync::Config {
        sync::Config {
            syncman_dir: TempDir::new().unwrap().path().to_owned(),
        }
    }
}
