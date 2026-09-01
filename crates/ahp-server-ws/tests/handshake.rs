use std::sync::Arc;

use ahp_server_core::openai_agent_model::{
    AgentModel, ModelError, ModelRequest, ModelRequestFuture,
};
use ahp_server_core::workspace_agent_tools::ConfiguredWorkspaces;
use ahp_server_core::{CoreConfig, CoreHost, MemoryHostStore, WorkspaceAgentProfile};
use axum::http::{header::ORIGIN, HeaderValue};
use tokio::net::TcpListener;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use ahp_server_ws::{router, WsOptions};

struct NeverModel;

impl AgentModel for NeverModel {
    fn model_name(&self) -> &str {
        "test"
    }

    fn stream<'a>(&'a self, _: &'a ModelRequest) -> ModelRequestFuture<'a> {
        Box::pin(async {
            Err(ModelError::InvalidResponse(
                "model should not run during an upgrade".into(),
            ))
        })
    }
}

fn host() -> CoreHost {
    CoreHost::new(
        CoreConfig::default(),
        Arc::new(MemoryHostStore::new()),
        Arc::new(NeverModel),
        Arc::new(WorkspaceAgentProfile::new(
            ConfiguredWorkspaces::from_current_dir().expect("workspace root"),
        )),
    )
    .expect("valid core host")
}

async fn connect(origin: Option<&str>, options: WsOptions) -> Result<(), u16> {
    let host = host();
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("listener address");
    let app = router(host.clone(), options).expect("valid options");
    let shutdown = host.shutdown();
    let task = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move { shutdown.cancelled().await })
            .await
            .expect("server");
    });
    let mut request = format!("ws://{address}/ahp")
        .into_client_request()
        .expect("upgrade request");
    if let Some(origin) = origin {
        let origin = if origin == "same-origin" {
            format!("http://{address}")
        } else {
            origin.into()
        };
        request.headers_mut().insert(
            ORIGIN,
            HeaderValue::from_str(&origin).expect("origin header"),
        );
    }
    let result = connect_async(request)
        .await
        .map(|(stream, _)| {
            drop(stream);
        })
        .map_err(|error| match error {
            tokio_tungstenite::tungstenite::Error::Http(response) => response.status().as_u16(),
            _ => 0,
        });
    host.shutdown().cancel();
    task.await.expect("server task");
    result
}

#[tokio::test]
async fn accepts_native_same_origin_and_configured_websocket_handshakes() {
    assert!(connect(None, WsOptions::default()).await.is_ok());
    assert!(connect(Some("same-origin"), WsOptions::default())
        .await
        .is_ok());
    let result = connect(
        Some("https://allowed.example"),
        WsOptions {
            allowed_origins: vec!["https://allowed.example".into()],
        },
    )
    .await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn rejects_disallowed_or_malformed_browser_origins() {
    assert_eq!(
        connect(Some("https://elsewhere.example"), WsOptions::default()).await,
        Err(403)
    );
    assert_eq!(
        connect(Some("https://elsewhere.example/path"), WsOptions::default()).await,
        Err(400)
    );
}

#[test]
fn rejects_invalid_configured_origins() {
    assert!(router(
        host(),
        WsOptions {
            allowed_origins: vec!["ftp://example.test".into()]
        }
    )
    .is_err());
}
