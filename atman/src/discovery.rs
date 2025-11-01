use actman::Handle;
use iroh::EndpointId;
use tokio::sync::oneshot;
use tracing::info;

use crate::{
    Error,
    actors::{network, sync},
    doc::{self, Document},
};

pub async fn get_local_node_id(network_handle: &Handle<network::Actor>) -> EndpointId {
    let (reply_sender, reply_receiver) = oneshot::channel();
    network_handle
        .send(network::Message::Status { reply_sender })
        .await;
    reply_receiver.await.expect("network actor must exist")
}

pub async fn save_local_node_id(
    local_node_id: EndpointId,
    sync_handle: &Handle<sync::Actor>,
) -> Result<(), Error> {
    // Load the nodes document stored in the sync actor
    let (msg, reply_receiver) = sync::message::GetMessage {
        doc_space: doc::protocol::DOC_SPACE.into(),
        doc_id: doc::protocol::nodes::DOC_ID.into(),
    }
    .into();
    sync_handle.send(msg).await;
    let mut nodes = match reply_receiver.await.expect("sync actor must exist") {
        Ok(Document::Nodes(nodes)) => nodes,
        Ok(_) => {
            return Err(Error::UnexpectedDocumentType);
        }
        Err(sync::Error::DocumentNotFound { .. }) => Default::default(),
        Err(e) => return Err(e.into()),
    };

    // Exit early if the local node is already registered
    let local_node_id = local_node_id.to_string();
    if nodes.nodes().any(|node| node.id == local_node_id) {
        info!("local node is already in the nodes document");
        return Ok(());
    }

    info!("adding local node to the nodes document");
    nodes.nodes.push(doc::protocol::node::Node {
        id: local_node_id,
        signature: String::new(),
    });
    let (msg, reply_receiver) = sync::message::UpdateMessage {
        doc_space: doc::protocol::DOC_SPACE.into(),
        doc_id: doc::protocol::nodes::DOC_ID.into(),
        data: Document::Nodes(nodes).serialize()?,
    }
    .into();
    sync_handle.send(msg).await;
    Ok(reply_receiver.await.expect("sync actor must exist")?)
}

#[cfg(test)]
mod tests {
    use std::env::temp_dir;

    use iroh::SecretKey;

    use super::*;

    #[test_log::test(tokio::test)]
    async fn test_save_local_node_id() {
        let local_node_id = SecretKey::from_bytes(&[0u8; 32]).public();
        let sync_actor = sync::Actor::new(sync::Config {
            syncman_dir: temp_dir().join("discovery_test_syncman"),
        })
        .unwrap();

        let mut runner = actman::Runner::new();
        let sync_handle = runner.run(sync_actor);

        // Add the local node ID to the empty nodes document,
        // and save it.
        save_local_node_id(local_node_id, &sync_handle)
            .await
            .unwrap();
        let nodes = load_nodes_doc(&sync_handle).await.unwrap();
        assert!(
            nodes
                .nodes()
                .any(|node| node.id == local_node_id.to_string())
        );
        assert_eq!(nodes.nodes().count(), 1);

        // Try again. The nodes document should remain unchanged.
        save_local_node_id(local_node_id, &sync_handle)
            .await
            .unwrap();
        assert!(
            nodes
                .nodes()
                .any(|node| node.id == local_node_id.to_string())
        );
        assert_eq!(nodes.nodes().count(), 1);

        runner.shutdown().await;
    }

    async fn load_nodes_doc(
        sync_handle: &Handle<sync::Actor>,
    ) -> Result<doc::protocol::nodes::Nodes, Error> {
        let (msg, reply_receiver) = sync::message::GetMessage {
            doc_space: doc::protocol::DOC_SPACE.into(),
            doc_id: doc::protocol::nodes::DOC_ID.into(),
        }
        .into();
        sync_handle.send(msg).await;
        match reply_receiver.await.expect("sync actor must exist")? {
            Document::Nodes(nodes) => Ok(nodes),
            _ => Err(Error::UnexpectedDocumentType),
        }
    }
}
