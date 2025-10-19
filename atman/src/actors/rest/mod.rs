mod error;

use std::{net::SocketAddr, time::Duration};

use axum::{Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::get};
use serde::{Deserialize, Serialize};
use tokio::{sync::oneshot, task::JoinHandle};
use tower_http::timeout::TimeoutLayer;
use tracing::{error, info, warn};

pub use crate::actors::rest::error::Error;
use crate::{actors::network, command::Status};

pub struct Actor {
    server_join_handle: JoinHandle<()>,
    shutdown_sender: oneshot::Sender<()>,
}

#[async_trait::async_trait]
impl actman::Actor for Actor {
    type Message = ();

    async fn run(mut self, mut state: actman::State<Self>) {
        loop {
            tokio::select! {
                Some(()) = state.message_receiver.recv() => {}
                Some(ctrl) = state.control_receiver.recv() => {
                    match ctrl {
                        actman::Control::Shutdown => {
                            info!("Actor received shutdown control.");
                            break;
                        },
                    }
                }
                else => {
                    warn!("All channels closed, terminating actor.");
                    break;
                }
            }
        }

        self.shutdown().await;
    }
}

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
struct ServerState {
    network_handle: actman::Handle<network::Actor>,
}

impl Actor {
    pub async fn new(
        config: &Config,
        network_handle: actman::Handle<network::Actor>,
    ) -> Result<Self, Error> {
        let router = Router::new()
            .route("/status", get(status))
            .layer(
                // Necessary for graceful shutdown
                TimeoutLayer::new(REQUEST_TIMEOUT),
            )
            .with_state(ServerState { network_handle });
        let listener = tokio::net::TcpListener::bind(config.addr)
            .await
            .map_err(|cause| Error::IO {
                message: "Failed to listen on port".to_string(),
                cause,
            })?;
        let (shutdown_sender, shutdown_receiver) = oneshot::channel();
        let server_join_handle = tokio::spawn(async move {
            info!(
                "starting REST server: {:?}",
                listener.local_addr().expect("local_addr never fails")
            );
            axum::serve(listener, router)
                .with_graceful_shutdown(async move { shutdown_signal(shutdown_receiver).await })
                .await
                .expect("Never fails")
        });

        Ok(Self {
            server_join_handle,
            shutdown_sender,
        })
    }

    async fn shutdown(self) {
        // Send shutdown signal to the axum server
        self.shutdown_sender
            .send(())
            .expect("shutdown receiver must exist");
        // Wait until the axum server task is terminated
        self.server_join_handle
            .await
            .expect("REST server task must be terminated without error");
        info!("REST server has been shut down.");
    }
}

/// A future to be passed to the [`axum::Serve::with_graceful_shutdown`].
/// When this future resolves, the axum server will start graceful shutdown.
async fn shutdown_signal(shutdown_receiver: oneshot::Receiver<()>) {
    shutdown_receiver
        .await
        .expect("shutdown sender never be dropped");
    info!("starting graceful shutdown for REST server...");
}

async fn status(State(state): State<ServerState>) -> impl IntoResponse {
    let (reply_sender, reply_receiver) = oneshot::channel();
    state
        .network_handle
        .send(network::Message::Status { reply_sender })
        .await;
    let Ok(node_id) = reply_receiver.await else {
        error!("failed to receive network status");
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    };
    Ok(Json(Status { node_id }))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub addr: SocketAddr,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        }
    }
}
