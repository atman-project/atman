use std::fmt::{self, Debug, Formatter};

use iroh::{
    NodeId,
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

use crate::actors::network::Error;

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
        let node_id = connection.remote_node_id()?;
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
        let node_id = connection.remote_node_id()?;
        info!("Accepted connection from {node_id}");

        // Our protocol is a simple request-response protocol, so we expect the
        // connecting peer to open a single bi-directional stream.
        let (mut send, mut recv) = connection.accept_bi().await?;
        info!("Accepted stream");

        if let Err(e) = perform_sync(&self.sync_actor_handle, &mut send, &mut recv, false).await {
            error!("Failed to sync: {e}");
            return Err(AcceptError::from_err(e));
        }

        // By calling `finish` on the send stream we signal that we will not send
        // anything further, which makes the receive stream on the other end
        // terminate.
        send.finish()?;

        // Wait until the remote closes the connection, which it does once it
        // received the response.
        connection.closed().await;
        info!("Connection closed by remote");
        Ok(())
    }

    pub async fn connect_and_spawn(
        node_id: NodeId,
        router: &Router,
        sync_actor_handle: &actman::Handle<crate::sync::Actor>,
    ) -> Result<(), Error> {
        let conn = router.endpoint().connect(node_id, Self::ALPN).await?;
        let (mut send_stream, mut recv_stream) = conn.open_bi().await?;
        info!("Stream opened");

        let sync_actor_handle = sync_actor_handle.clone();
        tokio::spawn(async move {
            perform_sync(&sync_actor_handle, &mut send_stream, &mut recv_stream, true)
                .await
                .inspect_err(|e| error!("Failed to perform sync: {e:?}"))
        });
        Ok(())
    }
}

async fn perform_sync<SendStream, RecvStream>(
    sync_actor_handle: &actman::Handle<crate::sync::Actor>,
    send_stream: &mut SendStream,
    recv_stream: &mut RecvStream,
    as_trigger: bool,
) -> Result<(), Error>
where
    SendStream: AsyncWriteExt + Unpin,
    RecvStream: AsyncReadExt + Unpin,
{
    let (reply_sender, reply_receiver) = oneshot::channel();
    sync_actor_handle
        .send(crate::sync::message::Message::InitiateSync { reply_sender })
        .await;
    let mut sync_handle = reply_receiver.await.expect("Sync actor must be available");

    if as_trigger && !generate_and_send_sync_msg(&mut sync_handle, send_stream).await? {
        return Ok(());
    }

    loop {
        match read_msg(recv_stream).await? {
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

        if !generate_and_send_sync_msg(&mut sync_handle, send_stream).await? {
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
        send_msg(&msg, send_stream).await?;
        Ok(true)
    } else {
        info!("Nothing to sync. Syncing has been done.");
        send_finish_msg(send_stream).await?;
        Ok(false)
    }
}

async fn send_msg<Stream>(msg: &[u8], stream: &mut Stream) -> Result<(), Error>
where
    Stream: AsyncWriteExt + Unpin,
{
    if msg.is_empty() {
        return send_finish_msg(stream).await;
    }

    assert!(!msg.is_empty());
    info!("Sending sync message to peer: len:{}", msg.len());
    stream.write_u64(msg.len() as u64).await?;
    stream.write_all(msg).await?;
    Ok(())
}

async fn send_finish_msg<Stream>(stream: &mut Stream) -> Result<(), Error>
where
    Stream: AsyncWriteExt + Unpin,
{
    info!("Sending sync finish message to peer");
    stream.write_u64(0u64).await?;
    Ok(())
}

async fn read_msg<Stream>(stream: &mut Stream) -> Result<Option<Vec<u8>>, Error>
where
    Stream: AsyncReadExt + Unpin,
{
    info!("Reading sync response from peer");
    let len = stream.read_u64().await?;
    if len == 0 {
        info!("Received zero length");
        return Ok(None);
    }
    let mut buf = vec![0u8; len as usize];
    stream.read_exact(&mut buf).await?;
    Ok(Some(buf))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Event {
    Accepted {
        node_id: NodeId,
    },
    Closed {
        node_id: NodeId,
        error: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use tempdir::TempDir;

    use super::*;
    use crate::actors::sync;

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
            perform_sync(
                &sync_handle_1,
                &mut listener_send,
                &mut listener_recv,
                false,
            )
            .await
        });
        // Run trigger side
        perform_sync(&sync_handle_0, &mut trigger_send, &mut trigger_recv, true)
            .await
            .unwrap();
        // Wait for listener to finish without error.
        let _ = join_listener.await.unwrap();

        runner.shutdown().await;
    }

    fn sync_config() -> sync::Config {
        sync::Config {
            syncman_dir: TempDir::new("network-sync-test").unwrap().into_path(),
            overwrite: false,
        }
    }
}
