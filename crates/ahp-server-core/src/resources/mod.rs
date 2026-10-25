//! Workspace-confined filesystem requests and short-lived resource watches.

mod access;
mod files;
mod operations;
mod watch;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use ahp_types::messages::JsonRpcRequest;
use ahp_types::state::{Snapshot, SnapshotState};
use serde_json::Value;

use crate::host::CoreHost;
use crate::rpc::RpcFault;

#[derive(Default)]
pub(crate) struct Resources {
    io: tokio::sync::Mutex<()>,
    versions: Mutex<HashMap<PathBuf, u64>>,
    watches: Mutex<HashMap<String, watch::Watch>>,
}

pub(crate) fn is_request(method: &str) -> bool {
    matches!(
        method,
        "resourceRead"
            | "resourceWrite"
            | "resourceList"
            | "resourceMkdir"
            | "resourceDelete"
            | "resourceMove"
            | "resourceCopy"
            | "resourceResolve"
            | "resourceRequest"
            | "createResourceWatch"
    )
}

pub(crate) async fn request(
    host: &CoreHost,
    peer_id: u64,
    request: JsonRpcRequest,
) -> Result<Value, RpcFault> {
    if request.method == "createResourceWatch" {
        return watch::create(host, peer_id, crate::rpc::params(&request)?).await;
    }
    let _io = host.inner.resources.io.lock().await;
    let host = host.clone();
    tokio::task::spawn_blocking(move || operations::request(&host, request))
        .await
        .map_err(RpcFault::internal)?
}

impl Resources {
    pub(crate) fn snapshot(&self, channel: &str, sequence: i64) -> Option<Snapshot> {
        self.watches
            .lock()
            .ok()?
            .get(channel)
            .map(|watch| Snapshot {
                resource: channel.into(),
                state: SnapshotState::ResourceWatch(Box::new(watch.state.clone())),
                from_seq: sequence,
            })
    }

    pub(crate) fn subscribe(&self, peer_id: u64, channel: &str) {
        if let Ok(mut watches) = self.watches.lock() {
            if let Some(watch) = watches.get_mut(channel) {
                watch.subscribers.insert(peer_id);
            }
        }
    }

    pub(crate) fn unsubscribe(&self, peer_id: u64, channel: &str) {
        if let Ok(mut watches) = self.watches.lock() {
            if let Some(watch) = watches.get_mut(channel) {
                watch.subscribers.remove(&peer_id);
                if watch.subscribers.is_empty() {
                    watches.remove(channel);
                }
            }
        }
    }

    pub(crate) fn close_peer(&self, peer_id: u64) {
        if let Ok(mut watches) = self.watches.lock() {
            watches.retain(|_, watch| {
                let removed = watch.subscribers.remove(&peer_id);
                !(watch.subscribers.is_empty() && (removed || watch.owner == peer_id))
            });
        }
    }

    pub(crate) fn clear(&self) {
        if let Ok(mut watches) = self.watches.lock() {
            watches.clear();
        }
    }
}
