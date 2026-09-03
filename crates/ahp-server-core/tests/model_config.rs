use std::sync::{Arc, Mutex};
use std::time::Duration;

use ahp_server_core::{
    CoreConfig, CoreHost, HostStore, MemoryHostStore, ModelCatalog, SnapshotRequest,
    WorkspaceAgentProfile,
};
use futures_util::stream;
use openai_agent_model::{
    AgentModel, ModelEventStream, ModelRequest, ModelRequestFuture, ModelStreamEvent,
    ReasoningEffort,
};
use serde_json::{json, Value};
use tokio::task::yield_now;
use tokio::time::timeout;
use workspace_agent_tools::ConfiguredWorkspaces;

struct RecordingModel {
    requests: Mutex<Vec<ModelRequest>>,
    default_model: Option<String>,
}

type HostFixture = (CoreHost, Arc<MemoryHostStore>, Arc<RecordingModel>);

impl RecordingModel {
    fn requests(&self) -> Vec<ModelRequest> {
        self.requests.lock().expect("model recorder mutex").clone()
    }
}

impl AgentModel for RecordingModel {
    fn default_model(&self) -> Option<&str> {
        self.default_model.as_deref()
    }

    fn stream<'a>(&'a self, request: &'a ModelRequest) -> ModelRequestFuture<'a> {
        self.requests
            .lock()
            .expect("model recorder mutex")
            .push(request.clone());
        Box::pin(async {
            let stream: ModelEventStream =
                Box::pin(stream::iter([Ok(ModelStreamEvent::Completed)]));
            Ok(stream)
        })
    }
}

fn catalog() -> ModelCatalog {
    let mut alpha = ModelCatalog::single("alpha").models.remove(0);
    alpha.name = "Alpha".into();
    let mut beta = ModelCatalog::single("beta").models.remove(0);
    beta.name = "Beta".into();
    ModelCatalog {
        models: vec![alpha, beta],
        default_selection: Some(ahp_types::state::ModelSelection {
            id: "alpha".into(),
            config: None,
        }),
    }
}

fn host() -> Result<HostFixture, Box<dyn std::error::Error>> {
    let store = Arc::new(MemoryHostStore::new());
    let model = Arc::new(RecordingModel {
        requests: Mutex::new(Vec::new()),
        default_model: Some("alpha".into()),
    });
    let host = host_with_catalog(catalog(), store.clone(), model.clone())?;
    Ok((host, store, model))
}

fn no_default_catalog() -> ModelCatalog {
    ModelCatalog::new([("alpha", "Alpha"), ("beta", "Beta")])
}

fn host_with_catalog(
    model_catalog: ModelCatalog,
    store: Arc<dyn HostStore>,
    model: Arc<RecordingModel>,
) -> Result<CoreHost, Box<dyn std::error::Error>> {
    let profile = WorkspaceAgentProfile::new(ConfiguredWorkspaces::from_current_dir()?);
    Ok(CoreHost::with_model_catalog(
        CoreConfig::default(),
        model_catalog,
        store,
        model,
        Arc::new(profile),
    )?)
}

#[test]
fn convenience_constructor_requires_an_explicit_catalog_without_an_adapter_default(
) -> Result<(), Box<dyn std::error::Error>> {
    let profile = WorkspaceAgentProfile::new(ConfiguredWorkspaces::from_current_dir()?);
    let result = CoreHost::new(
        CoreConfig::default(),
        Arc::new(MemoryHostStore::new()),
        Arc::new(RecordingModel {
            requests: Mutex::new(Vec::new()),
            default_model: None,
        }),
        Arc::new(profile),
    );
    assert!(matches!(
        result,
        Err(ahp_server_core::CoreHostError::ExplicitModelCatalogRequired)
    ));
    Ok(())
}

async fn next(peer: &mut ahp_server_core::CorePeer) -> Result<Value, Box<dyn std::error::Error>> {
    let frame = timeout(Duration::from_secs(1), peer.recv())
        .await?
        .ok_or("peer closed")?;
    Ok(serde_json::from_str(&frame)?)
}

async fn request(
    host: &CoreHost,
    peer: &mut ahp_server_core::CorePeer,
    id: u64,
    method: &str,
    params: Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    host.handle_text(
        peer.id(),
        &json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }).to_string(),
    )
    .await?;
    loop {
        let frame = next(peer).await?;
        if frame.get("id") == Some(&Value::from(id)) {
            return Ok(frame);
        }
    }
}

async fn notify(
    host: &CoreHost,
    peer: &ahp_server_core::CorePeer,
    channel: &str,
    client_seq: i64,
    action: Value,
) -> Result<(), Box<dyn std::error::Error>> {
    host.handle_text(
        peer.id(),
        &json!({
            "jsonrpc": "2.0", "method": "dispatchAction",
            "params": { "channel": channel, "clientSeq": client_seq, "action": action }
        })
        .to_string(),
    )
    .await?;
    Ok(())
}

async fn action(
    peer: &mut ahp_server_core::CorePeer,
    expected_type: &str,
) -> Result<Value, Box<dyn std::error::Error>> {
    loop {
        let frame = next(peer).await?;
        if frame.pointer("/params/action/type").and_then(Value::as_str) == Some(expected_type) {
            return Ok(frame);
        }
    }
}

async fn wait_for_requests(
    model: &RecordingModel,
    count: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    for _ in 0..100 {
        if model.requests().len() >= count {
            return Ok(());
        }
        yield_now().await;
    }
    Err(format!("model did not receive {count} requests").into())
}

#[tokio::test]
async fn advertises_and_resolves_the_complete_catalog() -> Result<(), Box<dyn std::error::Error>> {
    let (host, _, _) = host()?;
    let mut peer = host.open_peer().await;
    let initialized = request(
        &host,
        &mut peer,
        1,
        "initialize",
        json!({
            "channel": "ahp-root://", "clientId": "client", "protocolVersions": ["0.8.0"],
            "initialSubscriptions": ["ahp-root://"]
        }),
    )
    .await?;
    assert_eq!(
        initialized
            .pointer("/result/snapshots/0/state/agents/0/models")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(2)
    );
    assert!(initialized
        .pointer(
            "/result/snapshots/0/state/agents/0/models/0/configSchema/properties/thinkingLevel"
        )
        .is_some());

    let resolved = request(
        &host,
        &mut peer,
        2,
        "resolveSessionConfig",
        json!({ "channel": "ahp-root://", "provider": "openai" }),
    )
    .await?;
    assert_eq!(
        resolved.pointer("/result/values/model/id"),
        Some(&json!("alpha"))
    );
    let partial = request(
        &host,
        &mut peer,
        3,
        "resolveSessionConfig",
        json!({
            "channel": "ahp-root://",
            "config": { "model": { "config": { "thinkingLevel": "medium" } } }
        }),
    )
    .await?;
    assert_eq!(
        partial.pointer("/result/values/model/config/thinkingLevel"),
        Some(&json!("medium"))
    );
    let invalid = request(
        &host,
        &mut peer,
        4,
        "resolveSessionConfig",
        json!({ "channel": "ahp-root://", "config": { "model": { "id": "missing" } } }),
    )
    .await?;
    assert_eq!(invalid.pointer("/error/code"), Some(&json!(-32602)));

    let completions = request(
        &host,
        &mut peer,
        5,
        "sessionConfigCompletions",
        json!({ "channel": "ahp-root://", "property": "model", "query": "ET" }),
    )
    .await?;
    assert_eq!(
        completions.pointer("/result/items/0/value"),
        Some(&json!("beta"))
    );
    let schema: ahp_types::state::SessionConfigSchema =
        serde_json::from_value(resolved["result"]["schema"].clone())?;
    assert_eq!(serde_json::to_value(schema)?, resolved["result"]["schema"]);
    Ok(())
}

#[tokio::test]
async fn requires_a_selection_when_the_host_has_no_default_model(
) -> Result<(), Box<dyn std::error::Error>> {
    let store = Arc::new(MemoryHostStore::new());
    let model = Arc::new(RecordingModel {
        requests: Mutex::new(Vec::new()),
        default_model: None,
    });
    let host = host_with_catalog(no_default_catalog(), store, model.clone())?;
    let mut peer = host.open_peer().await;
    request(
        &host,
        &mut peer,
        1,
        "initialize",
        json!({
            "channel": "ahp-root://", "clientId": "client", "protocolVersions": ["0.8.0"],
            "initialSubscriptions": ["ahp-root://"]
        }),
    )
    .await?;

    let resolved = request(
        &host,
        &mut peer,
        2,
        "resolveSessionConfig",
        json!({ "channel": "ahp-root://", "provider": "openai" }),
    )
    .await?;
    assert!(resolved.pointer("/result/values/model").is_none());
    assert_eq!(
        resolved.pointer("/result/schema/required"),
        Some(&json!(["model"]))
    );
    assert!(resolved
        .pointer("/result/schema/properties/model/default")
        .is_none());

    let partial = request(
        &host,
        &mut peer,
        8,
        "resolveSessionConfig",
        json!({
            "channel": "ahp-root://",
            "config": { "model": { "id": "beta" } }
        }),
    )
    .await?;
    assert_eq!(
        partial.pointer("/result/values/model/id"),
        Some(&json!("beta"))
    );

    let unresolved_partial = request(
        &host,
        &mut peer,
        9,
        "resolveSessionConfig",
        json!({
            "channel": "ahp-root://",
            "config": { "model": { "config": { "thinkingLevel": "medium" } } }
        }),
    )
    .await?;
    assert!(unresolved_partial.pointer("/result/values/model").is_none());
    assert_eq!(
        unresolved_partial.pointer("/result/schema/required"),
        Some(&json!(["model"]))
    );

    let unconfigured = request(
        &host,
        &mut peer,
        3,
        "createSession",
        json!({ "channel": "ahp-session:/missing-model" }),
    )
    .await?;
    assert_eq!(unconfigured["result"], Value::Null);

    let created = request(
        &host,
        &mut peer,
        4,
        "createSession",
        json!({
            "channel": "ahp-session:/selected-model",
            "config": { "model": { "id": "beta" } }
        }),
    )
    .await?;
    assert_eq!(created["result"], Value::Null);
    let session = request(
        &host,
        &mut peer,
        5,
        "subscribe",
        json!({ "channel": "ahp-session:/selected-model" }),
    )
    .await?;
    assert_eq!(
        session.pointer("/result/snapshot/state/config/values/model/id"),
        Some(&json!("beta"))
    );
    let chat = session["result"]["snapshot"]["state"]["defaultChat"]
        .as_str()
        .ok_or("missing default chat")?
        .to_owned();
    request(&host, &mut peer, 6, "subscribe", json!({ "channel": chat })).await?;
    notify(
        &host,
        &peer,
        &chat,
        7,
        json!({
            "type": "chat/turnStarted", "turnId": "selected",
            "startedAt": "2026-09-01T00:00:00.000Z",
            "message": { "text": "selected", "origin": { "kind": "user" } }
        }),
    )
    .await?;
    wait_for_requests(&model, 1).await?;
    assert_eq!(
        model.requests()[0]
            .model_selection
            .as_ref()
            .map(|selection| selection.id.as_str()),
        Some("beta")
    );
    host.shutdown_and_wait().await?;
    Ok(())
}

#[path = "model_config/draft.rs"]
mod model_config_draft;
#[path = "model_config/explicit_precedence.rs"]
mod model_config_explicit_precedence;
#[path = "model_config/history.rs"]
mod model_config_history;
#[path = "model_config/stability.rs"]
mod model_config_stability;
#[path = "model_config/stale.rs"]
mod model_config_stale;
#[path = "model_config/turns.rs"]
mod model_config_turns;
