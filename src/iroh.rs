use iroh::{
    Endpoint, NodeId, Watcher,
    endpoint::Connection,
    protocol::{AcceptError, ProtocolHandler, Router},
};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tracing::{error, info};

pub struct Iroh {
    router: Router,
}

impl Iroh {
    pub async fn new() -> Result<Self, Error> {
        let endpoint = Endpoint::builder()
            .discovery_n0()
            .discovery_local_network()
            .alpns(vec![Echo::ALPN.to_vec()])
            .bind()
            .await?;
        let (event_sender, _) = broadcast::channel(128);
        let echo = Echo::new(event_sender.clone());
        let router = Router::builder(endpoint).accept(Echo::ALPN, echo).spawn();
        match router.endpoint().node_addr().initialized().await {
            Ok(addr) => info!("Node address initialized: {addr:?}"),
            Err(e) => error!("Failed to watch for node address to be initialized: {e:?}"),
        }
        Ok(Self { router })
    }

    pub async fn connect(&self, node_id: NodeId) -> Result<(), Error> {
        let conn = self.router.endpoint().connect(node_id, Echo::ALPN).await?;
        let (mut send_stream, mut recv_stream) = conn.open_bi().await?;
        info!("Stream opened");
        tokio::spawn(async move {
            info!("Writing data to stream");
            if let Err(e) = send_stream.write_all(b"hello echo").await {
                error!("Failed to send data: {e}");
                return;
            }
            if let Err(e) = send_stream.finish() {
                error!("Failed to finish send_stream: {e}");
                return;
            }
            info!("Reading data from stream");
            match recv_stream.read_to_end(1024).await {
                Ok(received) => {
                    info!("Received data: {:?}", String::from_utf8_lossy(&received));
                }
                Err(e) => {
                    error!("Failed to read stream: {e}");
                }
            }
            info!("Closing conn");
            conn.close(1u8.into(), b"done");
        });
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Bind error: {0}")]
    Bind(#[from] iroh::endpoint::BindError),
    #[error("Connect error: {0}")]
    Connect(#[from] iroh::endpoint::ConnectError),
    #[error("Connection error: {0}")]
    Connection(#[from] iroh::endpoint::ConnectionError),
}

#[derive(Debug, Clone)]
pub struct Echo {
    event_sender: broadcast::Sender<AcceptEvent>,
}

impl Echo {
    pub const ALPN: &[u8] = b"atman/echo/0";

    pub fn new(event_sender: broadcast::Sender<AcceptEvent>) -> Self {
        Self { event_sender }
    }
}

impl Echo {
    async fn handle_connection(self, connection: Connection) -> Result<(), AcceptError> {
        // Wait for the connection to be fully established.
        let node_id = connection.remote_node_id()?;
        self.event_sender
            .send(AcceptEvent::Accepted { node_id })
            .ok();
        let res = self.handle_connection_0(&connection).await;
        let error = res.as_ref().err().map(|err| err.to_string());
        self.event_sender
            .send(AcceptEvent::Closed { node_id, error })
            .ok();
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

        // Echo any bytes received back directly.
        let bytes_sent = tokio::io::copy(&mut recv, &mut send).await?;
        info!("Copied over {bytes_sent} byte(s)");

        // By calling `finish` on the send stream we signal that we will not send anything
        // further, which makes the receive stream on the other end terminate.
        send.finish()?;

        // Wait until the remote closes the connection, which it does once it
        // received the response.
        connection.closed().await;
        info!("Connection closed by remote");
        Ok(())
    }
}

impl ProtocolHandler for Echo {
    /// The `accept` method is called for each incoming connection for our ALPN.
    ///
    /// The returned future runs on a newly spawned tokio task, so it can run as long as
    /// the connection lasts.
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        self.clone().handle_connection(connection).await
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AcceptEvent {
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
