use std::fmt::{self, Debug, Formatter};

use iroh::{
    NodeId,
    endpoint::{Connection, RecvStream, SendStream},
    protocol::{AcceptError, ProtocolHandler, Router},
};
use serde::{Deserialize, Serialize};
use syncman::SyncHandle;
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

        if let Err(e) = self.sync(&mut send, &mut recv).await {
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

    async fn sync(
        &self,
        send_stream: &mut iroh::endpoint::SendStream,
        recv_stream: &mut iroh::endpoint::RecvStream,
    ) -> Result<(), Error> {
        let (reply_sender, reply_receiver) = oneshot::channel();
        self.sync_actor_handle
            .send(crate::sync::message::Message::InitiateSync { reply_sender })
            .await;
        let mut sync_handle = reply_receiver.await.expect("Sync actor must be available");

        loop {
            match read_msg(recv_stream).await {
                Ok(Some(msg)) => {
                    info!("Received sync message of length {}", msg.len());
                    let (reply_sender, reply_receiver) = oneshot::channel();
                    self.sync_actor_handle
                        .send(crate::sync::message::Message::ApplySync {
                            data: msg,
                            handle: sync_handle,
                            reply_sender,
                        })
                        .await;
                    let new_sync_handle = reply_receiver.await.expect("as")?;
                    sync_handle = new_sync_handle;
                }
                Ok(None) => {
                    info!("No more sync messages from peer. Syncing has been done.");
                    return Ok(());
                }
                Err(e) => {
                    error!("Failed to read sync response: {e:?}");
                    return Err(e);
                }
            }

            if let Some(msg) = sync_handle.generate_message() {
                if let Err(e) = send_msg(&msg, send_stream).await {
                    error!("Failed to send sync message: {e:?}");
                    return Err(e);
                }
            } else {
                info!("Nothing to sync. Syncing has been done.");
                if let Err(e) = send_finish_msg(send_stream).await {
                    error!("Failed to send sync finish message: {e:?}");
                }
                return Ok(());
            }
        }
    }

    pub async fn connect_and_spawn(
        node_id: NodeId,
        router: &Router,
        sync_actor_handle: &actman::Handle<crate::sync::Actor>,
    ) -> Result<(), Error> {
        let conn = router.endpoint().connect(node_id, Self::ALPN).await?;
        let (send_stream, recv_stream) = conn.open_bi().await?;
        info!("Stream opened");

        let sync_actor_handle = sync_actor_handle.clone();
        tokio::spawn(async move {
            perform_sync(sync_actor_handle, send_stream, recv_stream)
                .await
                .inspect_err(|e| error!("Failed to perform sync: {e:?}"))
        });
        Ok(())
    }
}

async fn perform_sync(
    sync_actor_handle: actman::Handle<crate::sync::Actor>,
    mut send_stream: SendStream,
    mut recv_stream: RecvStream,
) -> Result<(), Error> {
    let (reply_sender, reply_receiver) = oneshot::channel();
    sync_actor_handle
        .send(crate::sync::message::Message::InitiateSync { reply_sender })
        .await;
    let mut sync_handle = reply_receiver.await.expect("Sync actor must be available");

    loop {
        if let Some(msg) = sync_handle.generate_message() {
            send_msg(&msg, &mut send_stream).await?;
        } else {
            info!("Nothing to sync. Syncing has been done.");
            send_finish_msg(&mut send_stream).await?;
        }

        match read_msg(&mut recv_stream).await? {
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
                let new_sync_handle = reply_receiver
                    .await
                    .expect("Sync actor must be available")?;
                sync_handle = new_sync_handle;
            }
            None => {
                info!("No more sync messages from peer. Syncing has been done.");
                return Ok(());
            }
        }
    }
}

async fn send_msg(msg: &[u8], stream: &mut SendStream) -> Result<(), Error> {
    if msg.is_empty() {
        return send_finish_msg(stream).await;
    }

    assert!(!msg.is_empty());
    info!("Sending sync message to peer: len:{}", msg.len());
    stream.write_u64(msg.len() as u64).await?;
    stream.write_all(msg).await?;
    Ok(())
}

async fn send_finish_msg(stream: &mut SendStream) -> Result<(), Error> {
    info!("Sending sync finish message to peer");
    stream.write_u64(0u64).await?;
    Ok(())
}

async fn read_msg(stream: &mut RecvStream) -> Result<Option<Vec<u8>>, Error> {
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
