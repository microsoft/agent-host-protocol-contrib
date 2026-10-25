use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ahp_types::errors::ahp_error_codes;
use ahp_types::messages::JsonRpcRequest;
use openai_agent_model::{AgentModel, ModelRequest, ModelRequestFuture};
use serde_json::{json, Value};
use workspace_agent_tools::ConfiguredWorkspaces;

use super::{access, operations};
use crate::{CoreConfig, CoreHost, CorePeer, MemoryHostStore, WorkspaceAgentProfile};

struct UnusedModel;

impl AgentModel for UnusedModel {
    fn default_model(&self) -> Option<&str> {
        Some("resource-test")
    }

    fn stream<'a>(&'a self, _: &'a ModelRequest) -> ModelRequestFuture<'a> {
        Box::pin(async {
            let events: openai_agent_model::ModelEventStream =
                Box::pin(futures_util::stream::empty());
            Ok(events)
        })
    }
}

struct Fixture {
    host: CoreHost,
    directory: PathBuf,
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let directory = std::env::current_dir()
            .expect("test directory")
            .join(format!(".ahp-resource-test-{}", uuid::Uuid::new_v4()));
        let root = directory.join("workspace");
        fs::create_dir_all(&root).expect("create test workspace");
        let host = CoreHost::new(
            CoreConfig::default(),
            Arc::new(MemoryHostStore::new()),
            Arc::new(UnusedModel),
            Arc::new(WorkspaceAgentProfile::new(
                ConfiguredWorkspaces::new(vec![root.clone()]).expect("configured roots"),
            )),
        )
        .expect("test host");
        Self {
            host,
            directory,
            root,
        }
    }

    fn uri(&self, name: &str) -> String {
        access::uri(&self.root.join(name))
            .map_err(|error| error.message)
            .expect("valid file URI")
    }

    fn request(&self, method: &str, params: Value) -> Result<Value, crate::rpc::RpcFault> {
        operations::request(&self.host, request(method, params))
    }

    fn call(&self, method: &str, params: Value) -> Value {
        self.request(method, params)
            .map_err(|error| format!("{}: {}", error.code, error.message))
            .expect("resource request succeeds")
    }

    fn write(&self, name: &str, data: &str, extra: Value) -> Value {
        let mut params = json!({ "channel": "ahp-root://", "uri": self.uri(name), "encoding": "utf-8", "data": data });
        params
            .as_object_mut()
            .expect("object")
            .extend(extra.as_object().expect("extra object").clone());
        self.call("resourceWrite", params)
    }

    fn resolve(&self, name: &str) -> Value {
        self.call(
            "resourceResolve",
            json!({ "channel": "ahp-root://", "uri": self.uri(name) }),
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.host.inner.resources.clear();
        fs::remove_dir_all(&self.directory).expect("remove test workspace");
    }
}

fn request(method: &str, params: Value) -> JsonRpcRequest {
    serde_json::from_value(json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }))
        .expect("request")
}

#[test]
fn reads_text_and_losslessly_falls_back_to_base64_for_binary() {
    let fixture = Fixture::new();
    fixture.write("text.txt", "café 🌍", json!({}));
    let text = fixture.call(
        "resourceRead",
        json!({ "channel": "ahp-root://", "uri": fixture.uri("text.txt") }),
    );
    assert_eq!(text["encoding"], "utf-8");
    assert_eq!(text["data"], "café 🌍");
    fixture.call("resourceWrite", json!({ "channel": "ahp-root://", "uri": fixture.uri("binary"), "data": "AP+A", "encoding": "base64" }));
    assert_eq!(
        fs::read(fixture.root.join("binary")).expect("binary"),
        [0, 255, 128]
    );
    let binary = fixture.call(
        "resourceRead",
        json!({ "channel": "ahp-root://", "uri": fixture.uri("binary"), "encoding": "utf-8" }),
    );
    assert_eq!(binary["encoding"], "base64");
    assert_eq!(binary["data"], "AP+A");
}

#[test]
fn writes_use_byte_offsets_preserve_suffixes_and_enforce_preconditions() {
    let fixture = Fixture::new();
    fixture.write("text", "café trailing", json!({}));
    fixture.write("text", "🌍", json!({ "mode": "truncate", "position": 5 }));
    fixture.write("text", " ", json!({ "mode": "append", "position": 4 }));
    fixture.write("text", "-", json!({ "mode": "insert", "position": 5 }));
    assert_eq!(
        fs::read_to_string(fixture.root.join("text")).expect("contents"),
        "café- 🌍"
    );
    let before = fixture.resolve("text")["etag"].clone();
    fixture.write("text", "replacement", json!({ "ifMatch": before }));
    assert_ne!(fixture.resolve("text")["etag"], before);
    for (extra, code) in [
        (json!({ "ifMatch": before }), ahp_error_codes::CONFLICT),
        (
            json!({ "createOnly": true }),
            ahp_error_codes::ALREADY_EXISTS,
        ),
    ] {
        let mut params = json!({ "channel": "ahp-root://", "uri": fixture.uri("text"), "data": "lost", "encoding": "utf-8" });
        params
            .as_object_mut()
            .expect("params")
            .extend(extra.as_object().expect("extra").clone());
        assert_eq!(
            fixture
                .request("resourceWrite", params)
                .expect_err("precondition failure")
                .code,
            code
        );
    }
    assert_eq!(
        fs::read_to_string(fixture.root.join("text")).expect("contents"),
        "replacement"
    );
    let token = fixture.resolve("text")["etag"].clone();
    fixture.write("text", "replacement", json!({}));
    assert_ne!(fixture.resolve("text")["etag"], token);
    assert_eq!(fixture.request("resourceWrite", json!({ "channel": "ahp-root://", "uri": fixture.uri("missing/file"), "data": "x", "encoding": "utf-8" })).expect_err("missing parent").code, ahp_error_codes::NOT_FOUND);
}

#[test]
fn directory_operations_and_transfers_preserve_error_semantics() {
    let fixture = Fixture::new();
    let directory = json!({ "channel": "ahp-root://", "uri": fixture.uri("nested/child") });
    assert_eq!(fixture.call("resourceMkdir", directory.clone()), json!({}));
    assert_eq!(fixture.call("resourceMkdir", directory), json!({}));
    fixture.write("source", "original", json!({ "createOnly": true }));
    fixture.write("target", "old", json!({}));
    let transfer = json!({ "channel": "ahp-root://", "source": fixture.uri("source"), "destination": fixture.uri("target"), "failIfExists": true });
    for method in ["resourceCopy", "resourceMove"] {
        assert_eq!(
            fixture
                .request(method, transfer.clone())
                .expect_err("exists")
                .code,
            ahp_error_codes::ALREADY_EXISTS
        );
    }
    fixture.call("resourceCopy", json!({ "channel": "ahp-root://", "source": fixture.uri("source"), "destination": fixture.uri("target") }));
    fixture.call("resourceMove", json!({ "channel": "ahp-root://", "source": fixture.uri("source"), "destination": fixture.uri("moved") }));
    assert!(!fixture.root.join("source").exists());
    assert_eq!(
        fs::read_to_string(fixture.root.join("target")).expect("copy"),
        "original"
    );
    let entries = fixture.call(
        "resourceList",
        json!({ "channel": "ahp-root://", "uri": fixture.uri("") }),
    );
    assert!(entries["entries"]
        .as_array()
        .expect("entries")
        .contains(&json!({ "name": "nested", "type": "directory" })));
    assert_eq!(
        fixture
            .request(
                "resourceMkdir",
                json!({ "channel": "ahp-root://", "uri": fixture.uri("target") })
            )
            .expect_err("file exists")
            .code,
        ahp_error_codes::ALREADY_EXISTS
    );
    assert!(fixture
        .request(
            "resourceDelete",
            json!({ "channel": "ahp-root://", "uri": fixture.uri("nested") })
        )
        .is_err());
    fixture.call(
        "resourceDelete",
        json!({ "channel": "ahp-root://", "uri": fixture.uri("nested"), "recursive": true }),
    );
    assert_eq!(
        fixture
            .request(
                "resourceResolve",
                json!({ "channel": "ahp-root://", "uri": fixture.uri("nested") })
            )
            .expect_err("missing")
            .code,
        ahp_error_codes::NOT_FOUND
    );
}

#[test]
fn copying_directories_overwrites_without_destroying_overlapping_sources() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.root.join("source/child")).expect("source tree");
    fixture.write("source/child/new", "new", json!({}));
    fs::create_dir(fixture.root.join("destination")).expect("destination");
    fixture.write("destination/stale", "stale", json!({}));
    fixture.call("resourceCopy", json!({ "channel": "ahp-root://", "source": fixture.uri("source"), "destination": fixture.uri("destination") }));
    assert!(!fixture.root.join("destination/stale").exists());
    assert_eq!(
        fs::read_to_string(fixture.root.join("destination/child/new")).expect("copied child"),
        "new"
    );
    for method in ["resourceCopy", "resourceMove"] {
        assert!(fixture.request(method, json!({ "channel": "ahp-root://", "source": fixture.uri("source/child"), "destination": fixture.uri("source") })).is_err());
    }
    assert_eq!(
        fs::read_to_string(fixture.root.join("source/child/new")).expect("preserved child"),
        "new"
    );
}

#[test]
fn resource_requests_are_confined_and_unknown_schemes_are_rejected() {
    let fixture = Fixture::new();
    let outside = access::uri(&fixture.directory.join("outside"))
        .map_err(|error| error.message)
        .expect("valid outside file URI");
    let denied = fixture
        .request(
            "resourceRequest",
            json!({ "channel": "ahp-root://", "uri": outside, "read": true }),
        )
        .expect_err("outside denied");
    assert_eq!(denied.code, ahp_error_codes::PERMISSION_DENIED);
    assert_eq!(
        denied.data.expect("permission data")["request"]["uri"],
        outside
    );
    assert!(fixture
        .request(
            "resourceRead",
            json!({ "channel": "ahp-root://", "uri": "https://example.invalid/file" })
        )
        .is_err());
    assert_eq!(
        fixture.call(
            "resourceRequest",
            json!({ "channel": "ahp-root://", "uri": fixture.uri("") })
        ),
        json!({})
    );
}

#[cfg(unix)]
#[test]
fn symlinks_follow_resolution_but_cannot_escape_workspace_roots() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    fixture.write("target", "target", json!({}));
    symlink("target", fixture.root.join("link")).expect("symlink");
    assert_eq!(fixture.resolve("link")["uri"], fixture.uri("target"));
    let link = fixture.call(
        "resourceResolve",
        json!({ "channel": "ahp-root://", "uri": fixture.uri("link"), "followSymlinks": false }),
    );
    assert_eq!(link["type"], "symlink");
    assert_eq!(link["uri"], fixture.uri("link"));
    fs::create_dir(fixture.directory.join("outside")).expect("outside");
    symlink(
        fixture.directory.join("outside"),
        fixture.root.join("escape"),
    )
    .expect("escape link");
    for method in [
        "resourceRead",
        "resourceResolve",
        "resourceMkdir",
        "resourceRequest",
    ] {
        assert_eq!(
            fixture
                .request(
                    method,
                    json!({ "channel": "ahp-root://", "uri": fixture.uri("escape/missing") })
                )
                .expect_err("escape denied")
                .code,
            ahp_error_codes::PERMISSION_DENIED
        );
    }
    assert_eq!(fixture.request("resourceWrite", json!({ "channel": "ahp-root://", "uri": fixture.uri("escape/missing"), "encoding": "utf-8", "data": "escape" })).expect_err("write escape denied").code, ahp_error_codes::PERMISSION_DENIED);
    assert!(!fixture.directory.join("outside/missing").exists());
}

async fn rpc(fixture: &Fixture, peer: &mut CorePeer, method: &str, params: Value) -> Value {
    let request = request(method, params);
    fixture
        .host
        .handle_text(
            peer.id(),
            &serde_json::to_string(&request).expect("request JSON"),
        )
        .await
        .expect("handle request");
    loop {
        let text = tokio::time::timeout(Duration::from_secs(5), peer.recv())
            .await
            .expect("response timeout")
            .expect("peer open");
        let value: Value = serde_json::from_str(&text).expect("response JSON");
        if value.get("id").is_some() {
            assert!(value.get("error").is_none(), "{value}");
            return value["result"].clone();
        }
    }
}

async fn initialized(fixture: &Fixture) -> CorePeer {
    let mut peer = fixture.host.open_peer().await;
    rpc(fixture, &mut peer, "initialize", json!({ "channel": "ahp-root://", "clientId": uuid::Uuid::new_v4().to_string(), "protocolVersions": ["1.0.0"] })).await;
    peer
}

#[tokio::test]
async fn concurrent_append_requests_do_not_clobber_each_other() {
    let fixture = Fixture::new();
    fixture.write("append", "", json!({}));
    let mut tasks = tokio::task::JoinSet::new();
    for i in 0..8 {
        let host = fixture.host.clone();
        let params = json!({ "channel": "ahp-root://", "uri": fixture.uri("append"), "encoding": "utf-8", "data": format!("{i}\n"), "mode": "append" });
        tasks.spawn(async move {
            super::request(&host, 0, request("resourceWrite", params))
                .await
                .map_err(|error| error.message)
                .expect("append request succeeds");
        });
    }
    while let Some(result) = tasks.join_next().await {
        result.expect("append task");
    }
    let contents = fs::read_to_string(fixture.root.join("append")).expect("append contents");
    let mut lines = contents.lines().collect::<Vec<_>>();
    lines.sort();
    assert_eq!(lines, ["0", "1", "2", "3", "4", "5", "6", "7"]);
}

#[tokio::test]
async fn watches_publish_typed_snapshots_ordered_actions_and_release_last_subscriber() {
    let fixture = Fixture::new();
    let mut peer = initialized(&fixture).await;
    let created = rpc(&fixture, &mut peer, "createResourceWatch", json!({ "channel": "ahp-root://", "uri": fixture.uri(""), "recursive": true, "includes": { "items": ["**/*.txt"] } })).await;
    let channel = created["channel"].as_str().expect("channel");
    let subscribed = rpc(
        &fixture,
        &mut peer,
        "subscribe",
        json!({ "channel": channel }),
    )
    .await;
    assert_eq!(subscribed["snapshot"]["state"]["root"], fixture.uri(""));
    assert_eq!(subscribed["snapshot"]["state"]["recursive"], true);
    let from_seq = subscribed["snapshot"]["fromSeq"]
        .as_u64()
        .expect("sequence");
    fs::write(fixture.root.join("new.txt"), "new").expect("external write");
    let event = tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let frame = peer.recv().await.expect("peer open");
            let value: Value = serde_json::from_str(&frame).expect("notification");
            if value["params"]["channel"] == channel
                && value["params"]["action"]["type"] == "resourceWatch/changed"
            {
                break value;
            }
        }
    })
    .await
    .expect("watch event");
    assert!(event["params"]["serverSeq"].as_u64().expect("sequence") > from_seq);
    assert!(event["params"]["action"]["changes"]["items"]
        .as_array()
        .expect("changes")
        .contains(&json!({ "uri": fixture.uri("new.txt"), "type": "added" })));
    let mut sequence = event["params"]["serverSeq"].as_u64().expect("sequence");
    for kind in ["updated", "deleted"] {
        if kind == "updated" {
            fs::write(fixture.root.join("new.txt"), "updated contents").expect("external update");
        } else {
            fs::remove_file(fixture.root.join("new.txt")).expect("external deletion");
        }
        let event = tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                let frame = peer.recv().await.expect("peer open");
                let value: Value = serde_json::from_str(&frame).expect("notification");
                if value["params"]["channel"] == channel
                    && value["params"]["action"]["changes"]["items"]
                        .as_array()
                        .is_some_and(|changes| {
                            changes
                                .contains(&json!({ "uri": fixture.uri("new.txt"), "type": kind }))
                        })
                {
                    break value;
                }
            }
        })
        .await
        .expect("watch update or deletion");
        let next = event["params"]["serverSeq"].as_u64().expect("sequence");
        assert!(next > sequence);
        sequence = next;
    }
    let mut second = initialized(&fixture).await;
    rpc(
        &fixture,
        &mut second,
        "subscribe",
        json!({ "channel": channel }),
    )
    .await;
    fixture.host.unsubscribe(peer.id(), channel).await;
    assert!(fixture
        .host
        .inner
        .resources
        .watches
        .lock()
        .expect("watches")
        .contains_key(channel));
    fixture.host.close_peer(second.id()).await;
    assert!(!fixture
        .host
        .inner
        .resources
        .watches
        .lock()
        .expect("watches")
        .contains_key(channel));
    fixture.host.shutdown_and_wait().await.expect("shutdown");
}

#[tokio::test]
async fn resource_io_does_not_hold_the_publication_gate() {
    let fixture = Fixture::new();
    fixture.write("read.txt", "contents", json!({}));
    let mut reader = initialized(&fixture).await;
    let mut other = initialized(&fixture).await;
    let io = fixture.host.inner.resources.io.lock().await;
    let host = fixture.host.clone();
    let peer_id = reader.id();
    let frame = serde_json::to_string(&request(
        "resourceRead",
        json!({ "channel": "ahp-root://", "uri": fixture.uri("read.txt") }),
    ))
    .expect("read frame");
    let reading = tokio::spawn(async move { host.handle_text(peer_id, &frame).await });
    tokio::task::yield_now().await;
    let ping = tokio::time::timeout(
        Duration::from_secs(1),
        rpc(
            &fixture,
            &mut other,
            "ping",
            json!({ "channel": "ahp-root://" }),
        ),
    )
    .await;
    drop(io);
    reading.await.expect("read task").expect("read request");
    assert!(ping.is_ok(), "resource I/O must not block other requests");
    let response = reader.recv().await.expect("read response");
    let value: Value = serde_json::from_str(&response).expect("response JSON");
    assert_eq!(value["result"]["data"], "contents");
    fixture.host.shutdown_and_wait().await.expect("shutdown");
}

#[cfg(unix)]
#[tokio::test]
async fn fifo_reads_writes_and_copies_fail_without_blocking() {
    use std::os::unix::fs::OpenOptionsExt;

    let fixture = Fixture::new();
    fixture.write("regular", "original", json!({}));
    fs::create_dir(fixture.root.join("directory")).expect("directory");
    let fifo = fixture.root.join("directory/fifo");
    assert!(std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("create FIFO")
        .success());
    for (method, params) in [
        (
            "resourceRead",
            json!({ "channel": "ahp-root://", "uri": fixture.uri("directory/fifo") }),
        ),
        (
            "resourceWrite",
            json!({ "channel": "ahp-root://", "uri": fixture.uri("directory/fifo"), "data": "data", "encoding": "utf-8", "mode": "append" }),
        ),
        (
            "resourceCopy",
            json!({ "channel": "ahp-root://", "source": fixture.uri("directory/fifo"), "destination": fixture.uri("copy") }),
        ),
        (
            "resourceCopy",
            json!({ "channel": "ahp-root://", "source": fixture.uri("regular"), "destination": fixture.uri("directory/fifo") }),
        ),
        (
            "resourceCopy",
            json!({ "channel": "ahp-root://", "source": fixture.uri("directory"), "destination": fixture.uri("directory-copy") }),
        ),
    ] {
        // Closing this endpoint also releases a blocking reader if the regression returns.
        let endpoint = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&fifo)
            .expect("FIFO endpoint");
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            super::request(&fixture.host, 0, request(method, params)),
        )
        .await;
        drop(endpoint);
        let response = result.expect("special-file requests must not block");
        assert_eq!(
            response.expect_err("special-file request must fail").code,
            ahp_types::errors::json_rpc_error_codes::INVALID_PARAMS
        );
    }
    assert_eq!(
        fs::read_to_string(fixture.root.join("regular")).expect("regular contents"),
        "original"
    );
    tokio::time::timeout(Duration::from_secs(1), fixture.host.shutdown_and_wait())
        .await
        .expect("shutdown must not block")
        .expect("shutdown");
}
