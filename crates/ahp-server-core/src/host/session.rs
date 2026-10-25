//! Active-client membership cleanup when a peer leaves a session.

use ahp_types::state::SnapshotState;

use super::CoreHost;
use crate::{ChangeSession, SessionMutation, SnapshotRequest};

impl CoreHost {
    pub(super) fn schedule_peer_cleanup(&self, peer_id: u64, client_id: Option<String>) {
        self.inner.resources.close_peer(peer_id);
        if let Some(client_id) = client_id {
            let host = self.clone();
            // Eviction can happen while the publisher owns the publication gate.
            tokio::spawn(async move {
                host.remove_active_client(&client_id, None).await;
            });
        }
    }

    pub(super) async fn remove_active_client(&self, client_id: &str, channel: Option<&str>) {
        let _guard = self.inner.publish_lock.lock().await;
        let sessions = match channel {
            Some(channel) => vec![channel.to_owned()],
            None => match self.inner.store.list_sessions() {
                Ok(sessions) => sessions
                    .into_iter()
                    .map(|session| session.resource)
                    .collect(),
                Err(_) => return,
            },
        };
        for session in sessions {
            if self.inner.peers.lock().await.values().any(|peer| {
                peer.client_id.as_deref() == Some(client_id)
                    && peer.subscriptions.contains(&session)
            }) {
                continue;
            }
            let snapshot = self.inner.store.snapshot(SnapshotRequest {
                channel: session.clone(),
                model_catalog: self.inner.model_catalog.clone(),
                server_tools: Vec::new(),
            });
            let Ok(snapshot) = snapshot else { continue };
            let SnapshotState::Session(state) = snapshot.state else {
                continue;
            };
            if !state
                .active_clients
                .iter()
                .any(|client| client.client_id == client_id)
            {
                continue;
            }
            if let Ok(transition) = self.inner.store.change_session(ChangeSession {
                session: session.clone(),
                mutation: SessionMutation::ActiveClientRemoved(client_id.to_owned()),
                origin: None,
            }) {
                self.fanout(&transition).await;
                crate::turns::fail_client_tools_locked(self, &session, client_id).await;
            }
        }
    }
}
