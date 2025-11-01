use iroh::{
    EndpointId,
    endpoint::{Connection, RecvStream, SendStream},
    protocol::Router,
};
use tokio::io::AsyncWriteExt;
use tracing::{error, info};

use crate::actors::network::Error;

pub mod echo;
pub mod sync;

/// Connect to a remote node and open a bidirectional stream.
async fn connect(
    node_id: EndpointId,
    router: &Router,
    alpn: &[u8],
) -> Result<(Connection, SendStream, RecvStream), Error> {
    let conn = router.endpoint().connect(node_id, alpn).await?;
    let (send_stream, recv_stream) = conn.open_bi().await?;
    Ok((conn, send_stream, recv_stream))
}

/// Close the connection and the send stream properly:
/// - flush and finish the send stream
/// - wait until the peer reads all data
/// - close the connection
async fn close_conn(conn: &Connection, send_stream: &mut SendStream, with_err: bool) {
    let code = if with_err { 1u8 } else { 0u8 };
    info!("closing dialer conn: code:{code}");
    if let Err(e) = send_stream.flush().await {
        error!("failed to flush the send stream: {e}");
    }
    if let Err(e) = send_stream.finish() {
        error!("failed to finish the send stream: {e}");
    }
    let _ = send_stream.stopped().await;
    conn.close(code.into(), b"");
}

/// Wait until the connection is closed by the remote peer.
async fn wait_conn_closed(conn: &Connection, send_stream: &mut SendStream) {
    info!("waiting for the connection to be closed by remote");
    if let Err(e) = send_stream.finish() {
        error!("failed to finish the send stream: {e}");
    }
    conn.closed().await;
    info!("connection closed by remote");
}
