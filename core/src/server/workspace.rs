use super::*;
use crate::conversations::StoreError;

pub(super) async fn dispatch(
    inner: &Arc<Inner>,
    method: &str,
    raw: &str,
    body: &[u8],
) -> Result<(u16, Value)> {
    let url = reqwest::Url::parse(&format!("http://localhost{raw}"))?;
    let path = url.path();
    if path == "/ping" && method == "GET" {
        return Ok((200, json!({"ok":true})));
    }
    if path == "/performance" && method == "GET" {
        let cluster = inner.cluster.status().await;
        let active = matches!(
            cluster.state,
            crate::cluster::ClusterState::Running | crate::cluster::ClusterState::Starting
        );
        let local_rss = cluster
            .server_pid
            .and_then(crate::performance::process_rss_mb);
        let peers = inner.peers.lock().await.clone();
        let assignments = if active {
            cluster.assignments.clone()
        } else {
            Vec::new()
        };
        let mut devices = vec![
            json!({"id":inner.hardware.id,"name":inner.hardware.name,"self":true,
            "rss_mb":local_rss,"sample_age_seconds":0,"source":"engine process RSS",
            "assigned_layers":assignments.iter().find(|a| a.rpc.is_none()).map(|a| a.layers).unwrap_or(0),
            "planned_mb":assignments.iter().find(|a| a.rpc.is_none()).map(|a| a.memory_mb).unwrap_or(0)}),
        ];
        for peer in peers {
            let assignment = assignments.iter().find(|a| a.device_id == peer.hw.id);
            let fresh = peer.last_seen.elapsed().as_secs() < 30
                && peer.worker.as_ref().is_some_and(|w| w.state == "running");
            devices.push(json!({"id":peer.hw.id,"name":peer.hw.name,"self":false,
                "rss_mb":if fresh { peer.worker.as_ref().and_then(|w| w.rss_mb) } else { None },
                "coordinator_rtt_ms":if fresh { peer.worker.as_ref().and_then(|w| w.coordinator_rtt_ms) } else { None },
                "sample_age_seconds":peer.last_seen.elapsed().as_secs(),"source":"phone-reported worker RSS",
                "assigned_layers":assignment.map(|a| a.layers).unwrap_or(0),"planned_mb":assignment.map(|a| a.memory_mb).unwrap_or(0)}));
        }
        return Ok((
            200,
            json!({"sampled_at":crate::conversations::now_ms(),"generations":inner.performance.snapshot(),
            "coordinator_rss_mb":crate::performance::process_rss_mb(std::process::id()),"devices":devices,
            "cluster_state":cluster.state,"model":cluster.model,"ctx":cluster.ctx,
            "benchmark":inner.benchmarks.snapshot()}),
        ));
    }
    if path.starts_with("/benchmarks") {
        return benchmark::dispatch(inner, method, path, body).await;
    }
    let store = inner.conversations.clone();
    let parts: Vec<String> = path
        .trim_matches('/')
        .split('/')
        .map(String::from)
        .collect();
    let method = method.to_string();
    let body: Value = if body.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(body).map_err(|_| StoreError(400, "invalid JSON".into()))?
    };
    let query = url
        .query_pairs()
        .find(|(key, _)| key == "query")
        .map(|(_, v)| v.into_owned())
        .unwrap_or_default();
    let format = url
        .query_pairs()
        .find(|(key, _)| key == "format")
        .map(|(_, v)| v.into_owned())
        .unwrap_or_else(|| "markdown".into());
    tokio::task::spawn_blocking(move || {
        let revision = || body["revision"].as_u64().ok_or_else(|| StoreError(400,"revision is required".into()));
        match (method.as_str(), parts.as_slice()) {
            ("GET", [root]) if root == "conversations" => Ok((200,json!({"store_id":store.id,"conversations":store.list(&query)?}))),
            ("POST", [root]) if root == "conversations" => {
                let title = body["title"].as_str().unwrap_or("New chat");
                let conversation = if let Some(imported) = body.get("messages") {
                    let messages = imported.as_array().ok_or_else(|| StoreError(400,"messages must be an array".into()))?.iter().map(|m| {
                        Ok(crate::conversations::Message { id:crate::conversations::new_id()?,role:m["role"].as_str().unwrap_or("").into(),
                            content:m["content"].as_str().ok_or_else(|| StoreError(400,"message content must be text".into()))?.into(),
                            model:m["model"].as_str().map(String::from),created_at:crate::conversations::now_ms(),interrupted:m["interrupted"].as_bool().unwrap_or(false) })
                    }).collect::<Result<Vec<_>>>()?;
                    store.import(title,messages)?
                } else { store.create(title)? };
                Ok((200,json!({"conversation":conversation})))
            },
            ("GET", [root, id]) if root == "conversations" => Ok((200,json!({"conversation":store.get(id)?}))),
            ("POST", [root, id]) if root == "conversations" => Ok((200,json!({"conversation":store.rename(id, revision()?, body["title"].as_str().unwrap_or(""))?}))),
            ("POST", [root, id, action]) if root == "conversations" && action == "delete" => { store.delete(id,revision()?)?; Ok((200,json!({"ok":true}))) },
            ("POST", [root, id, action]) if root == "conversations" && action == "cancel" => Ok((200,json!({"conversation":store.cancel(id,body["request_id"].as_str().unwrap_or(""))?}))),
            ("GET", [root, id, action]) if root == "conversations" && action == "export" => {
                let conversation = store.get(id)?;
                let (extension, mime, content) = match format.as_str() {
                    "json" => ("json","application/json",serde_json::to_string_pretty(&conversation)?),
                    "markdown" => ("md","text/markdown",conversation.markdown()),
                    _ => return Err(StoreError(400,"format must be json or markdown".into()).into()),
                };
                Ok((200,json!({"filename":format!("localhostai-chat-{id}.{extension}"),"mime":mime,"content":content})))
            }
            _ => Ok((404,json!({"error":"not found"}))),
        }
    }).await?
}
