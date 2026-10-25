use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use ahp_types::actions::ResourceWatchChangedAction;
use ahp_types::commands::{CreateResourceWatchParams, CreateResourceWatchResult};
use ahp_types::state::{ResourceChange, ResourceChangeType, ResourceWatchState};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::{access, files};
use crate::host::CoreHost;
use crate::rpc::RpcFault;

pub(super) struct Watch {
    pub(super) state: ResourceWatchState,
    pub(super) owner: u64,
    pub(super) subscribers: HashSet<u64>,
    _watcher: RecommendedWatcher,
    cancellation: CancellationToken,
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

struct Filters {
    includes: Option<GlobSet>,
    excludes: Option<GlobSet>,
}

impl Filters {
    fn new(includes: &Option<Value>, excludes: &Option<Value>) -> Result<Self, RpcFault> {
        Ok(Self {
            includes: globs(includes)?,
            excludes: globs(excludes)?,
        })
    }

    fn matches(&self, path: &Path) -> bool {
        self.includes.as_ref().is_none_or(|set| set.is_match(path))
            && !self.excludes.as_ref().is_some_and(|set| set.is_match(path))
    }
}

fn globs(value: &Option<Value>) -> Result<Option<GlobSet>, RpcFault> {
    let Some(value) = value else {
        return Ok(None);
    };
    let items = value
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| RpcFault::invalid_params("watch filters must contain an items array"))?;
    let mut builder = GlobSetBuilder::new();
    for item in items {
        let pattern = item
            .as_str()
            .ok_or_else(|| RpcFault::invalid_params("watch filters must be strings"))?;
        let glob = GlobBuilder::new(pattern)
            .literal_separator(true)
            .build()
            .map_err(|error| RpcFault::invalid_params(error.to_string()))?;
        builder.add(glob);
    }
    builder
        .build()
        .map(Some)
        .map_err(|error| RpcFault::invalid_params(error.to_string()))
}

pub(super) async fn create(
    host: &CoreHost,
    peer_id: u64,
    params: CreateResourceWatchParams,
) -> Result<Value, RpcFault> {
    access::root(&params.channel)?;
    let path = access::path(host, &params.uri, false, true)?;
    fs::metadata(&path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::PermissionDenied {
            access::denied(&params.uri, false)
        } else {
            access::io_fault(error)
        }
    })?;
    let filters = Filters::new(&params.includes, &params.excludes)?;
    let recursive = params.recursive.unwrap_or(false);
    let channel = format!("ahp-resource-watch:/{}", uuid::Uuid::new_v4());
    let state = ResourceWatchState {
        root: params.uri,
        recursive,
        includes: params.includes,
        excludes: params.excludes,
    };
    let (events, receiver) = mpsc::channel(64);
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
        let _ = events.try_send(event);
    })
    .map_err(RpcFault::internal)?;
    watcher
        .watch(
            &path,
            if recursive {
                RecursiveMode::Recursive
            } else {
                RecursiveMode::NonRecursive
            },
        )
        .map_err(|error| match error.kind {
            notify::ErrorKind::Io(error) => {
                if error.kind() == std::io::ErrorKind::PermissionDenied {
                    access::denied(&state.root, false)
                } else {
                    access::io_fault(error)
                }
            }
            _ => RpcFault::internal(error),
        })?;
    let scan_root = path.clone();
    let scan = tokio::task::spawn_blocking(move || inventory(&scan_root, recursive));
    let inventory = tokio::select! {
        _ = host.inner.shutdown.cancelled() => {
            return Err(RpcFault::invalid_params("host is shutting down"));
        }
        result = scan => result.map_err(RpcFault::internal)?,
    };
    let peers = host.inner.peers.lock().await;
    if !peers.contains_key(&peer_id) || host.inner.shutdown.token().is_cancelled() {
        return Err(RpcFault::invalid_params("watch connection is closed"));
    }
    let cancellation = CancellationToken::new();
    host.inner
        .resources
        .watches
        .lock()
        .map_err(RpcFault::internal)?
        .insert(
            channel.clone(),
            Watch {
                state,
                owner: peer_id,
                subscribers: HashSet::new(),
                _watcher: watcher,
                cancellation: cancellation.clone(),
            },
        );
    drop(peers);
    let weak = Arc::downgrade(&host.inner);
    let shutdown = host.inner.shutdown.token().clone();
    let watch_channel = channel.clone();
    tokio::spawn(async move {
        run(
            weak.clone(),
            watch_channel.clone(),
            path,
            recursive,
            filters,
            inventory,
            receiver,
            cancellation,
            shutdown,
        )
        .await;
        if let Some(inner) = weak.upgrade() {
            if let Ok(mut watches) = inner.resources.watches.lock() {
                watches.remove(&watch_channel);
            };
        }
    });
    serde_json::to_value(CreateResourceWatchResult { channel }).map_err(RpcFault::internal)
}

#[allow(clippy::too_many_arguments)]
async fn run(
    weak: std::sync::Weak<crate::host::Inner>,
    channel: String,
    root: PathBuf,
    recursive: bool,
    filters: Filters,
    mut previous: BTreeMap<PathBuf, String>,
    mut receiver: mpsc::Receiver<notify::Result<Event>>,
    cancellation: CancellationToken,
    shutdown: CancellationToken,
) {
    loop {
        let first = tokio::select! {
            _ = cancellation.cancelled() => return,
            _ = shutdown.cancelled() => return,
            event = receiver.recv() => match event { Some(event) => event, None => return },
        };
        let mut events = vec![first];
        let debounce = tokio::time::sleep(Duration::from_millis(40));
        tokio::pin!(debounce);
        loop {
            tokio::select! {
                _ = cancellation.cancelled() => return,
                _ = shutdown.cancelled() => return,
                _ = &mut debounce => break,
                event = receiver.recv() => match event { Some(event) => events.push(event), None => return },
            }
        }
        let scan_root = root.clone();
        let scan = tokio::task::spawn_blocking(move || inventory(&scan_root, recursive));
        let current = tokio::select! {
            _ = cancellation.cancelled() => return,
            _ = shutdown.cancelled() => return,
            result = scan => match result { Ok(current) => current, Err(_) => return },
        };
        let mut changes = diff(&previous, &current);
        // A filesystem can preserve metadata on an in-place rewrite.
        for event in events.into_iter().flatten() {
            if matches!(event.kind, EventKind::Modify(_)) {
                for path in event.paths {
                    if previous.contains_key(&path) && current.contains_key(&path) {
                        changes.entry(path).or_insert(ResourceChangeType::Updated);
                    }
                }
            }
        }
        previous = current;
        let changes = changes
            .into_iter()
            .filter_map(|(path, r#type)| {
                let relative = path.strip_prefix(&root).ok()?;
                if !recursive && relative.components().count() > 1 {
                    return None;
                }
                if !filters.matches(relative) {
                    return None;
                }
                Some(ResourceChange {
                    uri: access::uri(&path).ok()?,
                    r#type,
                })
            })
            .collect::<Vec<_>>();
        if changes.is_empty() {
            continue;
        }
        let Some(inner) = weak.upgrade() else {
            return;
        };
        let host = CoreHost { inner };
        let _publication = host.inner.publish_lock.lock().await;
        let subscribed = host
            .inner
            .resources
            .watches
            .lock()
            .ok()
            .and_then(|watches| {
                watches
                    .get(&channel)
                    .map(|watch| !watch.subscribers.is_empty())
            })
            .unwrap_or(false);
        if !subscribed {
            continue;
        }
        let action = ResourceWatchChangedAction {
            changes: json!({ "items": changes }),
        };
        if let Ok(transition) = host
            .inner
            .store
            .publish_resource_change(channel.clone(), action)
        {
            host.fanout(&transition).await;
        }
    }
}

fn inventory(root: &Path, recursive: bool) -> BTreeMap<PathBuf, String> {
    fn scan(path: &Path, descend: bool, entries: &mut BTreeMap<PathBuf, String>) {
        let Ok(metadata) = fs::symlink_metadata(path) else {
            return;
        };
        entries.insert(path.to_owned(), files::fingerprint(&metadata));
        if metadata.is_dir() && descend {
            if let Ok(children) = fs::read_dir(path) {
                for child in children.flatten() {
                    scan(&child.path(), true, entries);
                }
            }
        }
    }
    let mut entries = BTreeMap::new();
    scan(root, recursive, &mut entries);
    if !recursive && root.is_dir() {
        if let Ok(children) = fs::read_dir(root) {
            for child in children.flatten() {
                scan(&child.path(), false, &mut entries);
            }
        }
    }
    entries
}

fn diff(
    previous: &BTreeMap<PathBuf, String>,
    current: &BTreeMap<PathBuf, String>,
) -> BTreeMap<PathBuf, ResourceChangeType> {
    let mut changes = BTreeMap::new();
    for (path, token) in current {
        match previous.get(path) {
            None => {
                changes.insert(path.clone(), ResourceChangeType::Added);
            }
            Some(old) if old != token => {
                changes.insert(path.clone(), ResourceChangeType::Updated);
            }
            _ => {}
        }
    }
    for path in previous.keys().filter(|path| !current.contains_key(*path)) {
        changes.insert(path.clone(), ResourceChangeType::Deleted);
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_match_relative_globs_with_excludes_taking_precedence() {
        let filters = Filters::new(
            &Some(json!({ "items": ["**/*.txt"] })),
            &Some(json!({ "items": ["ignored/**"] })),
        )
        .map_err(|error| error.message)
        .expect("valid filters");
        assert!(filters.matches(Path::new("note.txt")));
        assert!(filters.matches(Path::new("nested/note.txt")));
        assert!(!filters.matches(Path::new("ignored/note.txt")));
        assert!(!filters.matches(Path::new("image.png")));
    }

    #[test]
    fn diff_classifies_added_updated_deleted_and_omits_unchanged() {
        let previous = BTreeMap::from([
            (PathBuf::from("old"), "1".into()),
            (PathBuf::from("edit"), "1".into()),
            (PathBuf::from("same"), "1".into()),
        ]);
        let current = BTreeMap::from([
            (PathBuf::from("new"), "1".into()),
            (PathBuf::from("edit"), "2".into()),
            (PathBuf::from("same"), "1".into()),
        ]);
        let changes = diff(&previous, &current);
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[Path::new("old")], ResourceChangeType::Deleted);
        assert_eq!(changes[Path::new("new")], ResourceChangeType::Added);
        assert_eq!(changes[Path::new("edit")], ResourceChangeType::Updated);
    }
}
