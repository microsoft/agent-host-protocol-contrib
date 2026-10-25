//! Logical peers, bounded fan-out, and host construction.

mod session;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use ahp_types::actions::{ActionEnvelope, StateAction};
use ahp_types::common::{Uri, ROOT_RESOURCE_URI};
use ahp_types::messages::{JsonRpcMessage, JsonRpcNotification};
use openai_agent_model::AgentModel;
use serde_json::{json, Value};
use thiserror::Error;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::config::{CoreConfig, CoreConfigError};
use crate::model_catalog::{ModelCatalog, ModelCatalogError};
use crate::profile::AgentProfile;
use crate::store::{ApprovalKey, CommittedTransition, HostStore, StoreError};

/// A cloneable explicit shutdown hook for host adapters.
#[derive(Clone, Debug)]
pub struct Shutdown(CancellationToken);

impl Shutdown {
    /// Signals all turn streams, approval waits, and profile executions to stop.
    pub fn cancel(&self) {
        self.0.cancel();
    }

    /// Waits until shutdown was requested.
    pub async fn cancelled(&self) {
        self.0.cancelled().await;
    }

    pub(crate) fn token(&self) -> &CancellationToken {
        &self.0
    }
}

/// Construction or processing failure that cannot be represented as JSON-RPC.
#[derive(Debug, Error)]
pub enum CoreHostError {
    /// Configuration rejected before peers were accepted.
    #[error(transparent)]
    Config(#[from] CoreConfigError),
    /// Model catalogue rejected before peers were accepted.
    #[error(transparent)]
    ModelCatalog(#[from] ModelCatalogError),
    /// A store failed while processing a request.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// The convenience constructor cannot discover models without a configured
    /// adapter default.
    #[error(
        "an explicit ModelCatalog is required when the model adapter has no configured default"
    )]
    ExplicitModelCatalogRequired,
    /// A protocol frame could not be serialized.
    #[error("JSON-RPC serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    /// A tracked turn task did not complete successfully during shutdown.
    #[error("tracked turn task failed: {0}")]
    TurnTask(#[source] tokio::task::JoinError),
}

/// A transport-independent host. Clone it into each adapter connection task.
#[derive(Clone)]
pub struct CoreHost {
    pub(crate) inner: Arc<Inner>,
}

pub(crate) struct Inner {
    pub(crate) config: CoreConfig,
    pub(crate) store: Arc<dyn HostStore>,
    pub(crate) model: Arc<dyn AgentModel>,
    pub(crate) model_catalog: ModelCatalog,
    pub(crate) profile: Arc<dyn AgentProfile>,
    pub(crate) resources: crate::resources::Resources,
    pub(crate) shutdown: Shutdown,
    pub(crate) peers: Mutex<HashMap<u64, PeerState>>,
    pub(crate) cancellations: Mutex<HashMap<TurnKey, CancellationToken>>,
    pub(crate) approvals: Mutex<HashMap<ApprovalKey, PendingApproval>>,
    pub(crate) client_tools: Mutex<HashMap<ApprovalKey, crate::turns::PendingClientTool>>,
    pub(crate) publish_lock: Mutex<()>,
    pub(crate) turn_tasks: Mutex<JoinSet<()>>,
    next_peer: AtomicU64,
}

pub(crate) struct PeerState {
    pub(crate) sender: mpsc::Sender<String>,
    pub(crate) initialized: bool,
    pub(crate) client_id: Option<String>,
    pub(crate) subscriptions: HashSet<Uri>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct TurnKey {
    pub(crate) chat: Uri,
    pub(crate) turn_id: String,
}

pub(crate) struct PendingApproval {
    pub(crate) session: Uri,
    pub(crate) waiter_id: uuid::Uuid,
    pub(crate) presentation: crate::presentation::ToolPresentation,
    pub(crate) command: workspace_agent_tools::ExactCommandApproval,
    pub(crate) sender: tokio::sync::oneshot::Sender<crate::turns::ApprovalDecision>,
}

/// One logical connection and its outbound JSON-RPC frame receiver.
pub struct CorePeer {
    id: u64,
    outbound: mpsc::Receiver<String>,
}

impl CorePeer {
    /// Logical peer identity supplied to [`CoreHost::handle_text`].
    #[must_use]
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Waits for the next outbound JSON-RPC text frame.
    pub async fn recv(&mut self) -> Option<String> {
        self.outbound.recv().await
    }

    /// Attempts to receive an already queued outbound frame.
    pub fn try_recv(&mut self) -> Result<String, mpsc::error::TryRecvError> {
        self.outbound.try_recv()
    }
}

impl CoreHost {
    /// Creates the lifecycle host and validates its transport-independent limits.
    pub fn new(
        config: CoreConfig,
        store: Arc<dyn HostStore>,
        model: Arc<dyn AgentModel>,
        profile: Arc<dyn AgentProfile>,
    ) -> Result<Self, CoreHostError> {
        Self::with_model_catalog(
            config,
            ModelCatalog::single(
                model
                    .default_model()
                    .ok_or(CoreHostError::ExplicitModelCatalogRequired)?,
            ),
            store,
            model,
            profile,
        )
    }

    /// Creates a host with an explicit catalogue of selectable OpenAI models.
    pub fn with_model_catalog(
        config: CoreConfig,
        model_catalog: ModelCatalog,
        store: Arc<dyn HostStore>,
        model: Arc<dyn AgentModel>,
        profile: Arc<dyn AgentProfile>,
    ) -> Result<Self, CoreHostError> {
        config.validate()?;
        model_catalog.validate()?;
        Ok(Self {
            inner: Arc::new(Inner {
                config,
                store,
                model,
                model_catalog,
                profile,
                resources: crate::resources::Resources::default(),
                shutdown: Shutdown(CancellationToken::new()),
                peers: Mutex::new(HashMap::new()),
                cancellations: Mutex::new(HashMap::new()),
                approvals: Mutex::new(HashMap::new()),
                client_tools: Mutex::new(HashMap::new()),
                publish_lock: Mutex::new(()),
                turn_tasks: Mutex::new(JoinSet::new()),
                next_peer: AtomicU64::new(1),
            }),
        })
    }

    /// Returns a cancellation hook shared with all active lifecycle work.
    #[must_use]
    pub fn shutdown(&self) -> Shutdown {
        self.inner.shutdown.clone()
    }

    /// Cancels lifecycle work and waits for every tracked turn task to stop.
    ///
    /// Adapters should call this after their transport has stopped accepting
    /// peers and before closing a store whose lifetime is held by this host.
    /// The method does not poll: cancellation-aware model streams, approval
    /// waiters, and profile executions are joined directly.
    pub async fn shutdown_and_wait(&self) -> Result<(), CoreHostError> {
        self.inner.shutdown.cancel();
        self.inner.resources.clear();
        // Wait for in-flight starts, then release the gate needed by cancelled turn tasks.
        drop(self.inner.publish_lock.lock().await);
        let mut tasks = self.inner.turn_tasks.lock().await;
        let mut failure = None;
        while let Some(result) = tasks.join_next().await {
            if let Err(error) = result {
                if failure.is_none() {
                    failure = Some(error);
                }
            }
        }
        match failure {
            Some(error) => Err(CoreHostError::TurnTask(error)),
            None => Ok(()),
        }
    }

    /// Registers one bounded logical peer.
    pub async fn open_peer(&self) -> CorePeer {
        let id = self.inner.next_peer.fetch_add(1, Ordering::Relaxed);
        let (sender, outbound) = mpsc::channel(self.inner.config.outbound_queue_capacity);
        self.inner.peers.lock().await.insert(
            id,
            PeerState {
                sender,
                initialized: false,
                client_id: None,
                subscriptions: HashSet::new(),
            },
        );
        CorePeer { id, outbound }
    }

    /// Removes peer subscriptions and releases its bounded outbound sender.
    pub async fn close_peer(&self, peer_id: u64) {
        let client_id = self
            .inner
            .peers
            .lock()
            .await
            .remove(&peer_id)
            .and_then(|peer| peer.client_id);
        self.inner.resources.close_peer(peer_id);
        if let Some(client_id) = client_id {
            self.remove_active_client(&client_id, None).await;
        }
    }

    /// Removes a single subscription from one peer.
    pub async fn unsubscribe(&self, peer_id: u64, channel: &str) {
        let client_id = {
            let mut peers = self.inner.peers.lock().await;
            peers.get_mut(&peer_id).and_then(|peer| {
                peer.subscriptions.remove(channel);
                peer.client_id.clone()
            })
        };
        self.inner.resources.unsubscribe(peer_id, channel);
        if let Some(client_id) = client_id {
            if channel.starts_with("ahp-session:/") {
                self.remove_active_client(&client_id, Some(channel)).await;
            }
        }
    }

    /// Parses and routes one inbound JSON-RPC text frame.
    pub async fn handle_text(&self, peer_id: u64, text: &str) -> Result<(), CoreHostError> {
        crate::rpc::handle_text(self, peer_id, text).await
    }

    pub(crate) async fn send_value(&self, peer_id: u64, value: Value) -> Result<(), CoreHostError> {
        self.send_text(peer_id, serde_json::to_string(&value)?)
            .await;
        Ok(())
    }

    pub(crate) async fn send_message(
        &self,
        peer_id: u64,
        message: JsonRpcMessage,
    ) -> Result<(), CoreHostError> {
        self.send_text(peer_id, serde_json::to_string(&message)?)
            .await;
        Ok(())
    }

    async fn send_text(&self, peer_id: u64, text: String) {
        let mut peers = self.inner.peers.lock().await;
        let delivered = peers
            .get(&peer_id)
            .is_some_and(|peer| peer.sender.try_send(text).is_ok());
        let evicted = if delivered {
            None
        } else {
            peers.remove(&peer_id)
        };
        drop(peers);
        if let Some(peer) = evicted {
            self.schedule_peer_cleanup(peer_id, peer.client_id);
        }
    }

    /// Fans out only envelopes returned by a successful durable transition.
    pub(crate) async fn fanout<T>(&self, transition: &CommittedTransition<T>) {
        for envelope in &transition.envelopes {
            self.broadcast_action(envelope).await;
            if envelope.rejection_reason.is_none()
                && matches!(
                    envelope.action,
                    StateAction::SessionChatUpdated(_)
                        | StateAction::SessionChatAdded(_)
                        | StateAction::SessionChatRemoved(_)
                        | StateAction::SessionTitleChanged(_)
                        | StateAction::SessionIsReadChanged(_)
                        | StateAction::SessionIsArchivedChanged(_)
                )
            {
                if let Ok(summary) = self.inner.store.session_summary(&envelope.channel) {
                    let changes = match &envelope.action {
                        StateAction::SessionTitleChanged(_) => json!({ "title": summary.title }),
                        StateAction::SessionIsReadChanged(_)
                        | StateAction::SessionIsArchivedChanged(_) => {
                            json!({ "status": summary.status })
                        }
                        _ => {
                            json!({ "status": summary.status, "modifiedAt": summary.modified_at, "chats": summary.chats, "defaultChat": summary.default_chat })
                        }
                    };
                    self.broadcast_notification(
                        ROOT_RESOURCE_URI,
                        "root/sessionSummaryChanged",
                        json!({ "channel": ROOT_RESOURCE_URI, "session": envelope.channel, "changes": changes }),
                    ).await;
                }
            }
        }
    }

    pub(crate) async fn broadcast_action(&self, envelope: &ActionEnvelope) {
        let params = match serde_json::to_value(envelope) {
            Ok(params) => params,
            Err(_) => return,
        };
        self.broadcast_notification(&envelope.channel, "action", params)
            .await;
    }

    pub(crate) async fn broadcast_notification(&self, channel: &str, method: &str, params: Value) {
        let frame =
            match serde_json::to_string(&JsonRpcMessage::Notification(JsonRpcNotification {
                jsonrpc: Default::default(),
                method: method.into(),
                params: Some(params),
            })) {
                Ok(frame) => frame,
                Err(_) => return,
            };
        let mut peers = self.inner.peers.lock().await;
        let mut evicted = Vec::new();
        peers.retain(|id, peer| {
            let retained = !peer.subscriptions.contains(channel)
                || peer.sender.try_send(frame.clone()).is_ok();
            if !retained {
                evicted.push((*id, peer.client_id.clone()));
            }
            retained
        });
        drop(peers);
        for (peer_id, client_id) in evicted {
            self.schedule_peer_cleanup(peer_id, client_id);
        }
    }

    pub(crate) async fn peer_initialized(&self, peer_id: u64) -> bool {
        self.inner
            .peers
            .lock()
            .await
            .get(&peer_id)
            .is_some_and(|peer| peer.initialized)
    }

    pub(crate) async fn peer_is_subscribed(&self, peer_id: u64, channel: &str) -> bool {
        self.inner
            .peers
            .lock()
            .await
            .get(&peer_id)
            .is_some_and(|peer| peer.subscriptions.contains(channel))
    }

    pub(crate) async fn set_peer_initialized(
        &self,
        peer_id: u64,
        client_id: String,
        subscriptions: impl IntoIterator<Item = Uri>,
    ) {
        if let Some(peer) = self.inner.peers.lock().await.get_mut(&peer_id) {
            peer.initialized = true;
            peer.client_id = Some(client_id);
            for channel in subscriptions {
                self.inner.resources.subscribe(peer_id, &channel);
                peer.subscriptions.insert(channel);
            }
        }
    }

    pub(crate) async fn peer_origin(
        &self,
        peer_id: u64,
        client_seq: i64,
    ) -> Option<ahp_types::actions::ActionOrigin> {
        self.inner
            .peers
            .lock()
            .await
            .get(&peer_id)
            .and_then(|peer| peer.client_id.clone())
            .map(|client_id| ahp_types::actions::ActionOrigin {
                client_id,
                client_seq,
            })
    }
}
