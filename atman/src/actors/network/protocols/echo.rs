use iroh::{
    NodeId,
    endpoint::{Connection, RecvStream, SendStream},
    protocol::{AcceptError, ProtocolHandler, Router},
};
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    sync::{mpsc, oneshot},
};
use tracing::{error, info};

use crate::actors::network::{
    Error,
    protocols::{close_conn, connect},
};

#[derive(Debug, Clone)]
pub struct Protocol {
    event_sender: mpsc::Sender<Event>,
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
    pub const ALPN: &[u8] = b"atman/echo/0";

    pub fn new(event_sender: mpsc::Sender<Event>) -> Self {
        Self { event_sender }
    }
}

impl Protocol {
    async fn handle_connection(self, connection: Connection) -> Result<(), AcceptError> {
        // Wait for the connection to be fully established.
        let node_id = connection.remote_node_id()?;
        if let Err(e) = self.event_sender.send(Event::Accepted { node_id }).await {
            error!("Failed to send echo event to the channel: {e:?}");
        }
        let res = self.handle_connection_0(&connection).await;
        let error = res.as_ref().err().map(|err| err.to_string());
        if let Err(e) = self
            .event_sender
            .send(Event::Closed { node_id, error })
            .await
        {
            error!("Failed to send echo event to the channel: {e:?}");
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

        if let Err(e) = Self::echo_back(&mut send, &mut recv).await {
            error!("Failed to echo back: {e}");
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

    async fn echo_back(
        send: &mut iroh::endpoint::SendStream,
        recv: &mut iroh::endpoint::RecvStream,
    ) -> Result<(), Error> {
        loop {
            // Read the length of the incoming message.
            let len = recv.read_u64().await?;
            if len == 0 {
                info!("Received zero length");
                return Ok(());
            }
            let mut buf = vec![0u8; len as usize];
            recv.read_exact(&mut buf).await?;
            info!("Echoing data back: {:?}", String::from_utf8_lossy(&buf));
            send.write_u64(len).await?;
            send.write_all(&buf).await?;
        }
    }

    pub async fn connect_and_spawn(
        node_id: NodeId,
        router: &Router,
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

        tokio::spawn(async move {
            let result = perform_echo(&mut send_stream, &mut recv_stream)
                .await
                .inspect_err(|e| error!("Failed to perform sync: {e:?}"));
            close_conn(&conn, &mut send_stream, result.is_err()).await;
            let _ = reply_sender
                .send(result)
                .inspect_err(|_| error!("failed to send reply"));
        });
    }
}

async fn perform_echo(
    send_stream: &mut SendStream,
    recv_stream: &mut RecvStream,
) -> Result<(), Error> {
    for i in 0..10 {
        info!("Writing data to stream: {i}");
        let data = format!("Hello echo {i}");
        send_stream.write_u64(data.len() as u64).await?;
        send_stream.write_all(data.as_bytes()).await?;

        info!("Reading data from stream");
        let len = recv_stream.read_u64().await?;
        info!("Received length: {len}");
        let mut buf = vec![0u8; len as usize];
        recv_stream.read_exact(&mut buf).await?;
        info!("Received data: {:?}", String::from_utf8_lossy(&buf));
    }

    send_stream.write_u64(0).await?;
    info!("Sent final length");
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Event {
    Accepted {
        node_id: NodeId,
    },
    Echoed {
        node_id: NodeId,
        bytes_sent: u64,
    },
    Closed {
        node_id: NodeId,
        error: Option<String>,
    },
}
