//! Protected-resource authentication contracts.
//!
//! Internal conformance-runner module.

use std::time::Duration;

use ahp_types::commands::InitializeResult;
use ahp_types::state::SnapshotState;
use ahp_types::SUPPORTED_PROTOCOL_VERSIONS;
use serde_json::json;
use uuid::Uuid;

use super::{ConformanceError, Probe, ScenarioCheck, ScenarioRun};

pub(super) async fn run(
    target: &str,
    timeout_duration: Duration,
    auth_token: Option<&str>,
) -> ScenarioRun {
    let mut run = ScenarioRun::new();
    let mut probe = match Probe::connect(target, timeout_duration).await {
        Ok(probe) => probe,
        Err(error) => return run.abort(error),
    };
    let initialized = match probe
        .initialize(
            &format!("authentication-{}", Uuid::new_v4()),
            vec![SUPPORTED_PROTOCOL_VERSIONS[0]],
        )
        .await
    {
        Ok(reply) => reply,
        Err(error) => return run.abort(error),
    };
    let initialized: InitializeResult = match Probe::decode_result(&initialized, "initialize") {
        Ok(initialized) => initialized,
        Err(error) => return run.abort(error),
    };
    let resources = match initialized.snapshots.iter().find_map(|snapshot| {
        if snapshot.resource != "ahp-root://" {
            return None;
        }
        let SnapshotState::Root(root) = &snapshot.state else {
            return None;
        };
        root.agents
            .first()
            .map(|agent| agent.protected_resources.clone().unwrap_or_default())
    }) {
        Some(resources) => resources,
        None => return run.abort("initialize root snapshot did not advertise an agent provider"),
    };

    let invalid = match probe
        .request(
            "authenticate",
            json!({
                "channel": "ahp-root://",
                "resource": "https://invalid.example/ahp-conformance",
                "token": "invalid-conformance-token",
            }),
        )
        .await
    {
        Ok(reply) => reply,
        Err(error) => return run.abort(error),
    };
    if invalid.get("error").is_some() {
        run.push(ScenarioCheck::passed(
            "authentication-invalid-resource",
            "authenticate rejected an unadvertised protected resource",
        ));
    } else {
        run.push(ScenarioCheck::failed(
            "authentication-invalid-resource",
            "authenticate accepted an unadvertised protected resource",
        ));
    }

    if resources.is_empty() {
        run.push(ScenarioCheck::not_applicable(
            "authentication-advertised-resource",
            "agent did not advertise protected resources",
        ));
        return run;
    }

    let Some(token) = auth_token else {
        run.push(ScenarioCheck::not_applicable(
            "authentication-advertised-resource",
            "agent advertised protected resources but no authentication token was supplied",
        ));
        return run;
    };
    for resource in resources {
        let reply = match probe
            .request(
                "authenticate",
                json!({
                    "channel": "ahp-root://",
                    "resource": resource.resource,
                    "token": token,
                }),
            )
            .await
        {
            Ok(reply) => reply,
            Err(error) => return run.abort(error),
        };
        if let Some(error) = reply.get("error") {
            run.push(ScenarioCheck::failed(
                "authentication-advertised-resource",
                ConformanceError::Options(format!(
                    "authenticate rejected an advertised protected resource: {error}"
                )),
            ));
            return run;
        }
        if reply.get("result").is_none() {
            run.push(ScenarioCheck::failed(
                "authentication-advertised-resource",
                "authenticate omitted its JSON-RPC result",
            ));
            return run;
        }
    }
    run.push(ScenarioCheck::passed(
        "authentication-advertised-resource",
        "authenticate accepted every advertised protected resource",
    ));
    run
}
