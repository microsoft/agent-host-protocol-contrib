use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use ahp_server_redb::RedbHostStore;
use axum::body::{to_bytes, Body};
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use openai_agent_model::{ModelApi, ModelConfig, ModelDescriptor, ModelError};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tower::ServiceExt;
use uuid::Uuid;

use super::{model_catalog, web, AdvancedHost, AdvancedHostConfig, AdvancedHostError};

struct Artifact(PathBuf);

impl Artifact {
    fn new(name: &str) -> Self {
        Self(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("target")
                .join(format!("{name}-{}", Uuid::new_v4())),
        )
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Artifact {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct CatalogServer {
    address: std::net::SocketAddr,
    server: tokio::task::JoinHandle<()>,
}

impl CatalogServer {
    async fn start(status: StatusCode, body: Value) -> Result<Self, std::io::Error> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let app = Router::new()
            .route("/v1/models", get(catalog_response))
            .with_state((status, body));
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok(Self { address, server })
    }

    fn base_url(&self) -> String {
        format!("http://{}", self.address)
    }
}

impl Drop for CatalogServer {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn catalog_response(
    State((status, body)): State<(StatusCode, Value)>,
) -> (StatusCode, Json<Value>) {
    (status, Json(body))
}

struct DelayedModel {
    address: std::net::SocketAddr,
    entered: Arc<Notify>,
    server: tokio::task::JoinHandle<()>,
}

impl DelayedModel {
    async fn start() -> Result<Self, std::io::Error> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let entered = Arc::new(Notify::new());
        let app = Router::new()
            .route("/v1/responses", post(delayed_responses))
            .with_state(entered.clone());
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok(Self {
            address,
            entered,
            server,
        })
    }

    fn base_url(&self) -> String {
        format!("http://{}", self.address)
    }
}

impl Drop for DelayedModel {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn delayed_responses(State(entered): State<Arc<Notify>>) -> StatusCode {
    entered.notify_one();
    std::future::pending().await
}

#[test]
fn catalog_mapping_uses_an_optional_fallback_and_sorts_discovered_models(
) -> Result<(), Box<dyn std::error::Error>> {
    let inserted_default = model_catalog(
        Some("configured-default"),
        vec![
            ModelDescriptor {
                id: "zeta".into(),
                owned_by: Some("example".into()),
            },
            ModelDescriptor {
                id: "alpha".into(),
                owned_by: None,
            },
        ],
    )?;
    assert_eq!(
        inserted_default
            .models
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>(),
        ["alpha", "configured-default", "zeta"]
    );
    assert_eq!(
        inserted_default
            .default_selection
            .as_ref()
            .map(|selection| selection.id.as_str()),
        Some("configured-default")
    );
    assert_eq!(
        inserted_default.models[1].name, "configured-default",
        "the identifier is a readable label when discovery has no display name"
    );
    assert!(
        inserted_default.models[1].config_schema.is_none(),
        "an unrecognized configured default remains selectable without a guessed capability"
    );

    let sorted_catalog = model_catalog(
        Some("middle"),
        vec![
            ModelDescriptor {
                id: "zeta".into(),
                owned_by: None,
            },
            ModelDescriptor {
                id: "middle".into(),
                owned_by: None,
            },
            ModelDescriptor {
                id: "alpha".into(),
                owned_by: None,
            },
        ],
    )?;
    assert_eq!(
        sorted_catalog
            .models
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>(),
        ["alpha", "middle", "zeta"]
    );
    let fixture_catalog = model_catalog(
        Some("fixture-model"),
        vec![
            ModelDescriptor {
                id: "fixture-reasoning".into(),
                owned_by: None,
            },
            ModelDescriptor {
                id: "fixture-model".into(),
                owned_by: None,
            },
            ModelDescriptor {
                id: "fixture-fast".into(),
                owned_by: None,
            },
        ],
    )?;
    assert_eq!(
        fixture_catalog
            .default_selection
            .as_ref()
            .and_then(|selection| selection.config.as_ref()),
        None
    );
    let thinking_levels = |model_id: &str| {
        let model = fixture_catalog
            .models
            .iter()
            .find(|model| model.id == model_id)
            .ok_or_else(|| std::io::Error::other("fixture model missing"))?;
        Ok::<_, std::io::Error>(
            serde_json::to_value(model)?
                .pointer("/configSchema/properties/thinkingLevel/enum")
                .cloned(),
        )
    };
    assert_eq!(
        thinking_levels("fixture-fast")?,
        Some(json!(["none", "minimal", "low"]))
    );
    assert_eq!(
        thinking_levels("fixture-model")?,
        Some(json!(["minimal", "low", "medium", "high"]))
    );
    assert_eq!(
        thinking_levels("fixture-reasoning")?,
        Some(json!(["low", "medium", "high", "xhigh", "max"]))
    );
    let explicit_capability_catalog = model_catalog(
        Some("gpt-5.4-mini"),
        vec![
            ModelDescriptor {
                id: "unknown-mini".into(),
                owned_by: None,
            },
            ModelDescriptor {
                id: "gpt-5.4-mini".into(),
                owned_by: None,
            },
        ],
    )?;
    let model_schema = |model_id: &str| {
        explicit_capability_catalog
            .models
            .iter()
            .find(|model| model.id == model_id)
            .and_then(|model| model.config_schema.as_ref())
            .map(serde_json::to_value)
            .transpose()
    };
    assert_eq!(
        model_schema("gpt-5.4-mini")?
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/thinkingLevel/enum")),
        Some(&json!(["none", "low", "medium", "high", "xhigh"]))
    );
    assert!(
        model_schema("unknown-mini")?.is_none(),
        "unknown model IDs must not inherit mini-model reasoning capabilities"
    );
    let no_default_catalog = model_catalog(
        None,
        vec![ModelDescriptor {
            id: "discovered-model".into(),
            owned_by: None,
        }],
    )?;
    assert!(no_default_catalog.default_selection.is_none());
    Ok(())
}

#[tokio::test]
async fn unsupported_model_catalog_route_falls_back_to_the_configured_default(
) -> Result<(), Box<dyn std::error::Error>> {
    let artifact = Artifact::new("unsupported-model-catalog");
    let workspace = artifact.path().join("workspace");
    std::fs::create_dir_all(&workspace)?;
    let catalog =
        CatalogServer::start(StatusCode::NOT_FOUND, json!({"error": "unsupported"})).await?;
    let host = AdvancedHost::start(AdvancedHostConfig {
        bind: "127.0.0.1:0".parse()?,
        database_path: artifact.path().join("host.redb"),
        model: ModelConfig::new(catalog.base_url()).with_default_model("configured-default"),
        discover_models: true,
        workspace_roots: vec![workspace],
        reset_shell_approvals: false,
        chunk_size: 16,
        allowed_origins: Vec::new(),
        web_root: None,
    })
    .await?;
    assert_eq!(
        host.model_summary(),
        "Models: 1 available; default configured-default"
    );
    host.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn discovered_models_can_require_a_session_selection(
) -> Result<(), Box<dyn std::error::Error>> {
    let artifact = Artifact::new("catalog-without-default");
    let workspace = artifact.path().join("workspace");
    std::fs::create_dir_all(&workspace)?;
    let catalog = CatalogServer::start(
        StatusCode::OK,
        json!({"data": [{"id": "zeta"}, {"id": "alpha"}]}),
    )
    .await?;
    let host = AdvancedHost::start(AdvancedHostConfig {
        bind: "127.0.0.1:0".parse()?,
        database_path: artifact.path().join("host.redb"),
        model: ModelConfig::new(catalog.base_url()),
        discover_models: true,
        workspace_roots: vec![workspace],
        reset_shell_approvals: false,
        chunk_size: 16,
        allowed_origins: Vec::new(),
        web_root: None,
    })
    .await?;
    assert_eq!(host.model_summary(), "Models: 2 available; no host default");
    host.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn unsupported_model_catalog_without_fallback_fails_before_database_creation(
) -> Result<(), Box<dyn std::error::Error>> {
    let artifact = Artifact::new("unsupported-catalog-without-default");
    let workspace = artifact.path().join("workspace");
    let database = artifact.path().join("host.redb");
    std::fs::create_dir_all(&workspace)?;
    let catalog =
        CatalogServer::start(StatusCode::NOT_FOUND, json!({"error": "unsupported"})).await?;
    let result = AdvancedHost::start(AdvancedHostConfig {
        bind: "127.0.0.1:0".parse()?,
        database_path: database.clone(),
        model: ModelConfig::new(catalog.base_url()),
        discover_models: true,
        workspace_roots: vec![workspace],
        reset_shell_approvals: false,
        chunk_size: 16,
        allowed_origins: Vec::new(),
        web_root: None,
    })
    .await;
    assert!(matches!(
        result,
        Err(AdvancedHostError::ModelCatalogUnavailable {
            status
        }) if status == StatusCode::NOT_FOUND.as_u16()
    ));
    assert!(!database.exists());
    Ok(())
}

#[tokio::test]
async fn model_catalog_errors_fail_startup_without_creating_a_database(
) -> Result<(), Box<dyn std::error::Error>> {
    let artifact = Artifact::new("model-catalog-error");
    let workspace = artifact.path().join("workspace");
    let database = artifact.path().join("host.redb");
    std::fs::create_dir_all(&workspace)?;
    let catalog = CatalogServer::start(
        StatusCode::UNAUTHORIZED,
        json!({"error": "authentication failed"}),
    )
    .await?;
    let result = AdvancedHost::start(AdvancedHostConfig {
        bind: "127.0.0.1:0".parse()?,
        database_path: database.clone(),
        model: ModelConfig::new(catalog.base_url()).with_default_model("configured-default"),
        discover_models: true,
        workspace_roots: vec![workspace],
        reset_shell_approvals: false,
        chunk_size: 16,
        allowed_origins: Vec::new(),
        web_root: None,
    })
    .await;
    assert!(matches!(
        result,
        Err(AdvancedHostError::Model(ModelError::Upstream { status, .. }))
            if status == StatusCode::UNAUTHORIZED
    ));
    assert!(!database.exists());
    Ok(())
}

#[tokio::test]
async fn malformed_model_catalog_fails_startup() -> Result<(), Box<dyn std::error::Error>> {
    let artifact = Artifact::new("malformed-model-catalog");
    let workspace = artifact.path().join("workspace");
    std::fs::create_dir_all(&workspace)?;
    let catalog = CatalogServer::start(StatusCode::OK, json!({"data": "not an array"})).await?;
    let result = AdvancedHost::start(AdvancedHostConfig {
        bind: "127.0.0.1:0".parse()?,
        database_path: artifact.path().join("host.redb"),
        model: ModelConfig::new(catalog.base_url()).with_default_model("configured-default"),
        discover_models: true,
        workspace_roots: vec![workspace],
        reset_shell_approvals: false,
        chunk_size: 16,
        allowed_origins: Vec::new(),
        web_root: None,
    })
    .await;
    assert!(matches!(
        result,
        Err(AdvancedHostError::Model(ModelError::InvalidResponse(_)))
    ));
    Ok(())
}

#[tokio::test]
async fn web_root_serves_assets_spa_and_preserves_ahp_route(
) -> Result<(), Box<dyn std::error::Error>> {
    let artifact = Artifact::new("built-web-root");
    std::fs::create_dir_all(artifact.path().join("assets"))?;
    std::fs::write(artifact.path().join("index.html"), "<h1>chat app</h1>")?;
    std::fs::write(artifact.path().join("assets/app.js"), "console.log('app')")?;
    let app = Router::new()
        .route("/ahp", get(|| async { "AHP transport" }))
        .merge(web::router(Some(artifact.path().to_path_buf())));

    let root = app
        .clone()
        .oneshot(Request::builder().uri("/").body(Body::empty())?)
        .await?;
    assert_eq!(root.status(), StatusCode::OK);
    assert_eq!(to_bytes(root.into_body(), 1024).await?, "<h1>chat app</h1>");

    let asset = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/assets/app.js")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(asset.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(asset.into_body(), 1024).await?,
        "console.log('app')"
    );

    let spa = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/sessions/example")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(spa.status(), StatusCode::OK);
    assert_eq!(to_bytes(spa.into_body(), 1024).await?, "<h1>chat app</h1>");

    let ahp = app
        .oneshot(Request::builder().uri("/ahp").body(Body::empty())?)
        .await?;
    assert_eq!(to_bytes(ahp.into_body(), 1024).await?, "AHP transport");
    Ok(())
}

#[tokio::test]
async fn missing_web_root_returns_build_guidance() -> Result<(), Box<dyn std::error::Error>> {
    let artifact = Artifact::new("missing-web-root");
    let app = web::router(Some(artifact.path().to_path_buf()));
    let response = app
        .oneshot(Request::builder().uri("/").body(Body::empty())?)
        .await?;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = String::from_utf8(to_bytes(response.into_body(), 8_192).await?.to_vec())?;
    assert!(body.contains("npm ci"));
    assert!(body.contains("npm run build"));
    Ok(())
}

#[tokio::test]
async fn starts_and_stops_on_an_ephemeral_port() -> Result<(), Box<dyn std::error::Error>> {
    let artifact = Artifact::new("ephemeral-host");
    let workspace = artifact.path().join("workspace");
    std::fs::create_dir_all(&workspace)?;
    let host = AdvancedHost::start(AdvancedHostConfig {
        bind: "127.0.0.1:0".parse()?,
        database_path: artifact.path().join("host.redb"),
        model: ModelConfig::new("http://127.0.0.1:9").with_default_model("test-model"),
        discover_models: false,
        workspace_roots: vec![workspace],
        reset_shell_approvals: false,
        chunk_size: 16,
        allowed_origins: Vec::new(),
        web_root: None,
    })
    .await?;
    let response = reqwest::get(host.http_url()).await?;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let (socket, _) = tokio_tungstenite::connect_async(host.websocket_url()).await?;
    drop(socket);
    host.shutdown().await?;
    Ok(())
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn send(socket: &mut Socket, value: Value) -> Result<(), Box<dyn std::error::Error>> {
    socket.send(Message::Text(value.to_string().into())).await?;
    Ok(())
}

async fn next(socket: &mut Socket) -> Result<Value, Box<dyn std::error::Error>> {
    let message = timeout(Duration::from_secs(2), socket.next())
        .await?
        .ok_or_else(|| std::io::Error::other("host closed before response"))??;
    Ok(serde_json::from_str(message.into_text()?.as_str())?)
}

async fn response(socket: &mut Socket, id: u64) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        if next(socket).await?.get("id") == Some(&Value::from(id)) {
            return Ok(());
        }
    }
}

async fn action(socket: &mut Socket, kind: &str) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        if next(socket)
            .await?
            .pointer("/params/action/type")
            .and_then(Value::as_str)
            == Some(kind)
        {
            return Ok(());
        }
    }
}

#[tokio::test]
async fn shutdown_drains_a_delayed_turn_before_releasing_redb(
) -> Result<(), Box<dyn std::error::Error>> {
    let artifact = Artifact::new("shutdown-drain");
    let workspace = artifact.path().join("workspace");
    let database = artifact.path().join("host.redb");
    std::fs::create_dir_all(&workspace)?;
    let model = DelayedModel::start().await?;
    let host = AdvancedHost::start(AdvancedHostConfig {
        bind: "127.0.0.1:0".parse()?,
        database_path: database.clone(),
        model: ModelConfig::new(model.base_url())
            .with_default_model("test-model")
            .with_api(ModelApi::Responses),
        discover_models: true,
        workspace_roots: vec![workspace],
        reset_shell_approvals: false,
        chunk_size: 16,
        allowed_origins: Vec::new(),
        web_root: None,
    })
    .await?;
    let (mut socket, _) = tokio_tungstenite::connect_async(host.websocket_url()).await?;
    send(
        &mut socket,
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "channel": "ahp-root://", "clientId": "shutdown-test",
                "protocolVersions": ["1.0.0"], "initialSubscriptions": ["ahp-root://"]
            }
        }),
    )
    .await?;
    response(&mut socket, 1).await?;

    let session = "ahp-session:/shutdown-test";
    let chat = "ahp-chat:/shutdown-test";
    send(
        &mut socket,
        json!({ "jsonrpc": "2.0", "id": 2, "method": "createSession", "params": { "channel": session } }),
    )
    .await?;
    response(&mut socket, 2).await?;
    send(
        &mut socket,
        json!({ "jsonrpc": "2.0", "id": 3, "method": "createChat", "params": { "channel": session, "chat": chat } }),
    )
    .await?;
    response(&mut socket, 3).await?;
    send(
        &mut socket,
        json!({ "jsonrpc": "2.0", "id": 4, "method": "subscribe", "params": { "channel": chat } }),
    )
    .await?;
    response(&mut socket, 4).await?;
    send(
        &mut socket,
        json!({
            "jsonrpc": "2.0", "method": "dispatchAction",
            "params": {
                "channel": chat, "clientSeq": 1, "action": {
                    "type": "chat/turnStarted", "turnId": "delayed-turn",
                    "startedAt": "2026-01-01T00:00:00.000Z",
                    "message": { "text": "wait for the model", "origin": { "kind": "user" } }
                }
            }
        }),
    )
    .await?;
    action(&mut socket, "chat/turnStarted").await?;
    timeout(Duration::from_secs(2), model.entered.notified()).await?;

    timeout(Duration::from_secs(2), host.shutdown()).await??;
    let renamed = artifact.path().join("host-renamed.redb");
    std::fs::rename(&database, &renamed)?;
    std::fs::remove_file(&renamed)?;
    let reopened = RedbHostStore::open(&database)?;
    reopened.close()?;
    Ok(())
}
