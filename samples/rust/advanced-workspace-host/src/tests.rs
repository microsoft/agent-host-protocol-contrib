use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use ahp_server_redb::RedbHostStore;
use axum::body::{to_bytes, Body};
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::routing::{get, post};
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use openai_agent_model::{ModelApi, ModelConfig};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tower::ServiceExt;
use uuid::Uuid;

use super::{web, AdvancedHost, AdvancedHostConfig};

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
        model: ModelConfig::new("http://127.0.0.1:9", "test-model"),
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
        model: ModelConfig::new(model.base_url(), "test-model").with_api(ModelApi::Responses),
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
                "protocolVersions": ["0.8.0"], "initialSubscriptions": ["ahp-root://"]
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
