use std::collections::HashSet;

use actman::Handle;
use iroh::NodeId;
use tokio::sync::oneshot;
use tracing::{error, info};

use crate::{
    Error,
    actors::{network, sync},
    doc::{self, DocId, DocSpace, Document},
    models::Node,
};

pub async fn handle_sync_tick<SyncActor, NetworkActor>(
    sync_handle: &Handle<SyncActor>,
    network_handle: &Handle<NetworkActor>,
    local_node_id: &NodeId,
) -> Result<usize, Error>
where
    SyncActor: actman::Actor<Message = sync::message::Message>,
    NetworkActor: actman::Actor<Message = network::Message>,
{
    let nodes = load_remote_nodes(sync_handle, local_node_id).await?;
    if nodes.is_empty() {
        return Ok(0);
    }

    let (reply_sender, reply_receiver) = oneshot::channel();
    sync_handle
        .send(sync::message::Message::ListDocuments { reply_sender })
        .await;
    let docs = reply_receiver.await.expect("sync actor must exist");

    let mut successful_syncs = 0;
    for (doc_space, doc_id) in docs {
        for node in &nodes {
            match request_sync(node.id, doc_space.clone(), doc_id.clone(), network_handle).await {
                Ok(()) => {
                    info!("Periodic sync successful for {doc_space:?}/{doc_id:?} with {node:?}");
                    successful_syncs += 1;
                }
                Err(e) => {
                    error!(
                        "Periodic sync failed for {doc_space:?}/{doc_id:?} with {node:?}. Skipping...: {e}"
                    );
                }
            }
        }
    }
    Ok(successful_syncs)
}

async fn load_remote_nodes<SyncActor>(
    sync_handle: &Handle<SyncActor>,
    local_node_id: &NodeId,
) -> Result<HashSet<Node>, Error>
where
    SyncActor: actman::Actor<Message = sync::message::Message>,
{
    let (reply_sender, reply_receiver) = oneshot::channel();
    sync_handle
        .send(sync::message::Message::Get {
            msg: sync::message::GetMessage {
                doc_space: doc::protocol::DOC_SPACE.into(),
                doc_id: doc::protocol::nodes::DOC_ID.into(),
            },
            reply_sender,
        })
        .await;
    let nodes = match reply_receiver.await.expect("sync actor must exist")? {
        Document::Nodes(nodes) => nodes,
        _ => {
            error!("unexpected document type for nodes document");
            return Err(Error::UnexpectedDocumentType);
        }
    };

    Ok(nodes
        .nodes()
        .filter_map(|node| match Node::try_from(node) {
            Ok(node) => {
                if node.id != *local_node_id {
                    Some(node)
                } else {
                    None
                }
            }
            Err(e) => {
                error!("failed to parse node id {}. skipping this...: {e}", node.id);
                None
            }
        })
        .collect())
}

async fn request_sync<NetworkActor>(
    node_id: NodeId,
    doc_space: DocSpace,
    doc_id: DocId,
    network_handle: &Handle<NetworkActor>,
) -> Result<(), network::Error>
where
    NetworkActor: actman::Actor<Message = network::Message>,
{
    let (reply_sender, reply_receiver) = oneshot::channel();
    network_handle
        .send(network::Message::ConnectAndSync {
            node_id,
            doc_space,
            doc_id,
            reply_sender,
        })
        .await;
    reply_receiver.await.expect("network actor must exist")
}

#[cfg(test)]
mod tests {
    use iroh::SecretKey;
    use tempfile::TempDir;
    use tokio::sync::mpsc;

    use super::*;

    #[test_log::test(tokio::test)]
    async fn sync_tick_when_no_nodes_doc() {
        let sync_actor = sync::Actor::new(sync_config()).unwrap();
        let (network_actor, network_message_receiver) = DummyNetworkActor::new();

        let mut runner = actman::Runner::new();
        let sync_handle = runner.run(sync_actor);
        let network_handle = runner.run(network_actor);

        let Error::Sync(sync::Error::DocumentNotFound { doc_space, doc_id }) =
            handle_sync_tick(&sync_handle, &network_handle, &derive_node_id(0))
                .await
                .unwrap_err()
        else {
            panic!("expected DocumentNotFound error");
        };
        assert_eq!(doc_space.as_str(), doc::protocol::DOC_SPACE);
        assert_eq!(doc_id.as_str(), doc::protocol::nodes::DOC_ID);
        assert!(network_message_receiver.is_empty());

        runner.shutdown().await;
    }

    #[test_log::test(tokio::test)]
    async fn sync_tick_when_empty_nodes_doc() {
        let sync_actor = sync::Actor::new(sync_config()).unwrap();
        let (network_actor, network_message_receiver) = DummyNetworkActor::new();

        let mut runner = actman::Runner::new();
        let sync_handle = runner.run(sync_actor);
        let network_handle = runner.run(network_actor);

        prepare_nodes_doc(&sync_handle, &[]).await;

        let successful_syncs = handle_sync_tick(&sync_handle, &network_handle, &derive_node_id(0))
            .await
            .unwrap();
        assert_eq!(successful_syncs, 0);
        assert!(network_message_receiver.is_empty());

        runner.shutdown().await;
    }

    #[test_log::test(tokio::test)]
    async fn sync_tick_single_doc() {
        let sync_actor = sync::Actor::new(sync_config()).unwrap();
        let (network_actor, mut network_message_receiver) = DummyNetworkActor::new();

        let mut runner = actman::Runner::new();
        let sync_handle = runner.run(sync_actor);
        let network_handle = runner.run(network_actor);

        // Prepare a nodes doc
        let local_node_id = derive_node_id(0);
        let mut remote_node_ids = HashSet::from([derive_node_id(1), derive_node_id(2)]);
        prepare_nodes_doc(
            &sync_handle,
            &remote_node_ids
                .iter()
                .copied()
                .chain(std::iter::once(local_node_id))
                .map(|id| Node { id })
                .collect::<Vec<_>>(),
        )
        .await;

        // Trigger handling a sync tick
        let successful_syncs = handle_sync_tick(&sync_handle, &network_handle, &local_node_id)
            .await
            .unwrap();
        assert_eq!(successful_syncs, 2);

        // Verify that sync requests were sent to one of the nodes
        let (node_id, doc_space, doc_id) = network_message_receiver.recv().await.unwrap();
        assert!(remote_node_ids.remove(&node_id));
        assert_eq!(doc_space.as_str(), doc::protocol::DOC_SPACE);
        assert_eq!(doc_id.as_str(), doc::protocol::nodes::DOC_ID);

        // Verify that sync requests were sent to the nodes left.
        let (node_id, doc_space, doc_id) = network_message_receiver.recv().await.unwrap();
        assert!(remote_node_ids.remove(&node_id));
        assert_eq!(doc_space.as_str(), doc::protocol::DOC_SPACE);
        assert_eq!(doc_id.as_str(), doc::protocol::nodes::DOC_ID);

        assert!(network_message_receiver.is_empty());

        runner.shutdown().await;
    }

    async fn prepare_nodes_doc<SyncActor>(sync_handle: &Handle<SyncActor>, nodes: &[Node])
    where
        SyncActor: actman::Actor<Message = sync::message::Message>,
    {
        let doc = doc::Document::Nodes(doc::protocol::nodes::Nodes {
            nodes: nodes.iter().map(doc::protocol::node::Node::from).collect(),
        });
        let (msg, reply_receiver) = sync::message::UpdateMessage {
            doc_space: doc::protocol::DOC_SPACE.into(),
            doc_id: doc::protocol::nodes::DOC_ID.into(),
            data: doc.serialize().unwrap(),
        }
        .into();
        sync_handle.send(msg).await;
        reply_receiver.await.unwrap().unwrap();
    }

    fn sync_config() -> sync::Config {
        sync::Config {
            syncman_dir: TempDir::new().unwrap().path().to_owned(),
        }
    }

    fn derive_node_id(key: u8) -> NodeId {
        SecretKey::from_bytes(&[key; 32]).public()
    }

    struct DummyNetworkActor {
        sync_message_forwarder: mpsc::Sender<(NodeId, DocSpace, DocId)>,
    }

    impl DummyNetworkActor {
        fn new() -> (Self, mpsc::Receiver<(NodeId, DocSpace, DocId)>) {
            let (sync_message_forwarder, sync_message_receiver) = mpsc::channel(10);
            (
                Self {
                    sync_message_forwarder,
                },
                sync_message_receiver,
            )
        }
    }

    #[async_trait::async_trait]
    impl actman::Actor for DummyNetworkActor {
        type Message = network::Message;

        async fn run(mut self, mut state: actman::State<Self>) {
            loop {
                tokio::select! {
                    Some(message) = state.message_receiver.recv() => {
                        self.handle_message(message).await;
                    }
                    Some(ctrl) = state.control_receiver.recv() => {
                        match ctrl {
                            actman::Control::Shutdown => {
                                break;
                            },
                        }
                    }
                }
            }
        }
    }

    impl DummyNetworkActor {
        async fn handle_message(&self, message: network::Message) {
            match message {
                network::Message::ConnectAndSync {
                    node_id,
                    doc_space,
                    doc_id,
                    reply_sender,
                } => {
                    let _ = reply_sender.send(Ok(()));
                    let _ = self
                        .sync_message_forwarder
                        .send((node_id, doc_space, doc_id))
                        .await;
                }
                _ => {
                    panic!("unexpected message type")
                }
            }
        }
    }
}
