mod error;
mod protocols;

use iroh::{Endpoint, NodeId, SecretKey, Watcher as _, protocol::Router};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error, info, warn};

pub use crate::actors::network::error::Error;
use crate::actors::network::protocols::{echo, sync};

pub struct Actor {
    router: Router,
    sync_actor_handle: actman::Handle<crate::sync::Actor>,
    echo_event_receiver: mpsc::Receiver<echo::Event>,
    sync_event_receiver: mpsc::Receiver<sync::Event>,
}

#[async_trait::async_trait]
impl actman::Actor for Actor {
    type Message = Message;

    async fn run(mut self, mut state: actman::State<Self>) {
        loop {
            tokio::select! {
                Some(message) = state.message_receiver.recv() => {
                    self.handle_message(message).await
                }
                Some(ctrl) = state.control_receiver.recv() => {
                    match ctrl {
                        actman::Control::Shutdown => {
                            info!("Actor received shutdown control.");
                            self.shutdown().await;
                            break;
                        },
                    }
                }
                Some(event) = self.echo_event_receiver.recv() => {
                    debug!("Echo event: {event:?}");
                }
                Some(event) = self.sync_event_receiver.recv() => {
                    debug!("Echo event: {event:?}");
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

const EVENT_CHANNEL_SIZE: usize = 128;

impl Actor {
    pub async fn new(
        config: &Config,
        sync_actor_handle: actman::Handle<crate::sync::Actor>,
    ) -> Result<Self, Error> {
        let mut builder = Endpoint::builder();
        if let Some(key) = &config.key {
            builder = builder.secret_key(key.clone());
        }
        let endpoint = builder
            .discovery_n0()
            .discovery_local_network()
            .alpns(vec![
                echo::Protocol::ALPN.to_vec(),
                sync::Protocol::ALPN.to_vec(),
            ])
            .bind()
            .await?;
        let (echo_event_sender, echo_event_receiver) = mpsc::channel(EVENT_CHANNEL_SIZE);
        let echo = echo::Protocol::new(echo_event_sender);
        let (sync_event_sender, sync_event_receiver) = mpsc::channel(EVENT_CHANNEL_SIZE);
        let sync = sync::Protocol::new(sync_event_sender, sync_actor_handle.clone());
        let router = Router::builder(endpoint)
            .accept(echo::Protocol::ALPN, echo)
            .accept(sync::Protocol::ALPN, sync)
            .spawn();
        match router.endpoint().node_addr().initialized().await {
            Ok(addr) => info!("Node address initialized: {addr:?}"),
            Err(e) => error!("Failed to watch for node address to be initialized: {e:?}"),
        }
        Ok(Self {
            router,
            sync_actor_handle,
            echo_event_receiver,
            sync_event_receiver,
        })
    }

    async fn shutdown(self) {
        info!("shutting down the network actor.");
        if let Err(e) = self.router.shutdown().await {
            error!("error while shutting down the network router: {e:?}");
        }
        info!("network actor shut down.");
    }

    async fn handle_message(&self, message: Message) {
        match message {
            Message::Echo {
                node_id,
                reply_sender,
            } => {
                let _ = reply_sender
                    .send(self.handle_echo_message(node_id).await)
                    .inspect_err(|e| error!("Failed to send reply: {e:?}"));
            }
            Message::Sync {
                node_id,
                reply_sender,
            } => {
                let _ = reply_sender
                    .send(self.handle_sync_message(node_id).await)
                    .inspect_err(|e| error!("Failed to send reply: {e:?}"));
            }
            Message::Status { reply_sender } => {
                let _ = reply_sender
                    .send(self.router.endpoint().node_id())
                    .inspect_err(|e| error!("Failed to send reply: {e:?}"));
            }
        }
    }

    async fn handle_echo_message(&self, node_id: NodeId) -> Result<(), Error> {
        debug!("Handling Echo message to {node_id}");
        echo::Protocol::connect_and_spawn(node_id, &self.router).await
    }

    async fn handle_sync_message(&self, node_id: NodeId) -> Result<(), Error> {
        debug!("Handling Sync message to {node_id}");
        sync::Protocol::connect_and_spawn(node_id, &self.router, &self.sync_actor_handle).await
    }
}

pub enum Message {
    Echo {
        node_id: NodeId,
        reply_sender: oneshot::Sender<Result<(), Error>>,
    },
    Sync {
        node_id: NodeId,
        reply_sender: oneshot::Sender<Result<(), Error>>,
    },
    Status {
        reply_sender: oneshot::Sender<NodeId>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub key: Option<SecretKey>,
}
