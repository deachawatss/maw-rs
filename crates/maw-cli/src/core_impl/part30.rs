use axum::{
    body::Bytes,
    extract::{ConnectInfo, Path as AxumPath, Query, State},
    http::{HeaderMap, Method, StatusCode, Uri},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{Arc, Mutex},
};

const DEFAULT_SERVE_PORT: u16 = 31745;
const DEFAULT_SERVE_BIND: &str = "127.0.0.1";
#[cfg(test)]
const NON_LOOPBACK_TEST_PEER: SocketAddr =
    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 10)), 49_152);

struct ServeState {
    cached_pubkey: Option<String>,
    workspaces: Mutex<WorkspaceStore>,
    requests: Mutex<RequestReplyStore>,
    #[cfg(test)]
    peer_addr_override: Option<SocketAddr>,
    #[cfg(test)]
    now_override: Option<i64>,
    #[cfg(test)]
    serve_core_state_override: Option<crate::serve_core::ServecoreSharedState>,
    trust_store_path: std::path::PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ServeArgs {
    host: String,
    port: u16,
    cached_pubkey: Option<String>,
}

fn run_serve_async(args: Vec<String>) -> Pin<Box<dyn Future<Output = CliOutput> + Send>> {
    Box::pin(async move { run_serve_async_impl(&args).await })
}

async fn run_serve_async_impl(raw_args: &[String]) -> CliOutput {
    let args = match parse_serve_args(raw_args) {
        Ok(args) => args,
        Err(message) => return serve_usage_error(&message),
    };
    if args.host != DEFAULT_SERVE_BIND {
        return CliOutput {
            code: 2,
            stdout: String::new(),
            stderr: "serve: native gateway binds 127.0.0.1 only; publish via nginx\n".to_owned(),
        };
    }
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), args.port);
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(error) => {
            return CliOutput {
                code: 1,
                stdout: String::new(),
                stderr: format!("serve: failed to bind {addr}: {error}\n"),
            }
        }
    };
    let local_addr = match listener.local_addr() {
        Ok(addr) => addr,
        Err(error) => {
            return CliOutput {
                code: 1,
                stdout: String::new(),
                stderr: format!("serve: failed to read bound address: {error}\n"),
            }
        }
    };
    let app = serve_router(ServeState {
        cached_pubkey: args.cached_pubkey.or_else(load_inbound_peer_pubkey),
        workspaces: Mutex::new(WorkspaceStore::default()),
        requests: Mutex::new(RequestReplyStore::default()),
        #[cfg(test)]
        peer_addr_override: None,
        #[cfg(test)]
        now_override: None,
        #[cfg(test)]
        serve_core_state_override: None,
        trust_store_path: trust_store_path(),
    });
    println!("maw-rs serve listening http://{local_addr}");
    match axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    {
        Ok(()) => CliOutput {
            code: 0,
            stdout: String::new(),
            stderr: String::new(),
        },
        Err(error) => CliOutput {
            code: 1,
            stdout: String::new(),
            stderr: format!("serve: server error: {error}\n"),
        },
    }
}

fn parse_serve_args(argv: &[String]) -> Result<ServeArgs, String> {
    let mut host = default_bind_host();
    let mut port = DEFAULT_SERVE_PORT;
    let mut cached_pubkey = None;
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--host" | "--bind" => {
                let value = argv
                    .get(index + 1)
                    .ok_or_else(|| "serve: missing --host value".to_owned())?;
                host.clone_from(value);
                index += 1;
            }
            "--port" => {
                let value = argv
                    .get(index + 1)
                    .ok_or_else(|| "serve: missing --port value".to_owned())?;
                port = value
                    .parse::<u16>()
                    .map_err(|_| "serve: --port must be 0..65535".to_owned())?;
                index += 1;
            }
            "--cached-pubkey" => {
                let value = argv
                    .get(index + 1)
                    .ok_or_else(|| "serve: missing --cached-pubkey value".to_owned())?;
                cached_pubkey = Some(value.clone());
                index += 1;
            }
            "--help" | "-h" => return Err(String::new()),
            value if value.starts_with("--host=") => value["--host=".len()..].clone_into(&mut host),
            value if value.starts_with("--bind=") => value["--bind=".len()..].clone_into(&mut host),
            value if value.starts_with("--port=") => {
                port = value["--port=".len()..]
                    .parse::<u16>()
                    .map_err(|_| "serve: --port must be 0..65535".to_owned())?;
            }
            value if value.starts_with("--cached-pubkey=") => {
                cached_pubkey = Some(value["--cached-pubkey=".len()..].to_owned());
            }
            value if value.starts_with('-') => return Err(format!("serve: unknown argument {value}")),
            value => return Err(format!("serve: unexpected argument {value}")),
        }
        index += 1;
    }
    Ok(ServeArgs {
        host,
        port,
        cached_pubkey,
    })
}

fn serve_usage_error(message: &str) -> CliOutput {
    let prefix = if message.is_empty() {
        String::new()
    } else {
        format!("{message}\n")
    };
    CliOutput {
        code: 2,
        stdout: String::new(),
        stderr: format!(
            "{prefix}usage: maw-rs serve [--host 127.0.0.1] [--port <port>] [--cached-pubkey <key>]\n"
        ),
    }
}

fn default_bind_host() -> String {
    DEFAULT_SERVE_BIND.to_owned()
}

fn serve_core_state(state: &ServeState) -> crate::serve_core::ServecoreSharedState {
    #[cfg(not(test))]
    let _ = state;
    #[cfg(test)]
    if let Some(state) = &state.serve_core_state_override {
        return state.clone();
    }
    let core = crate::serve_core::ServecoreSharedState::default()
        .servecore_with_agents_node(load_hey_config().node)
        .servecore_with_auth(state.cached_pubkey.clone(), state.cached_pubkey.clone());
    #[cfg(not(test))]
    let core = core.servecore_with_process_auth_pins();
    #[cfg(test)]
    let core = if let Some(now) = state.now_override {
        core.servecore_with_auth_now(now)
    } else {
        core
    };
    core
}

fn serve_router(state: ServeState) -> Router {
    let serve_core_state = serve_core_state(&state);
    let state = Arc::new(state);
    let router = Router::new();
    let router = crate::serve_core::servecore_mount_core_routes(router);
    let router = crate::serve_core::servecore_mount_ws_routes(router);
    let router = crate::serve_core::modules::servecore_mount_modules(router, &[]);
    let router = router
        .route("/api/send", post(api_send))
        .route("/api/feed", get(api_feed_get).post(api_feed_post))
        .route("/api/sessions", get(api_sessions))
        .route("/api/capture", get(api_capture))
        .route("/api/probe", post(api_probe))
        .route("/api/wake", post(api_wake))
        .route("/api/pane-keys", post(api_pane_keys))
        .route("/api/transport/status", get(api_transport_status))
        .route("/api/transport/send", post(api_transport_send))
        .route("/api/health", get(api_health))
        .route("/api/message-ledger", get(api_message_ledger))
        .route("/api/requests", get(api_requests))
        .route("/api/trust", get(api_trust_list).post(api_trust_add))
        .route("/api/trust/revoke", post(api_trust_revoke))
        .route("/api/request", post(api_request_create))
        .route("/api/reply/:correlation_id", post(api_reply))
        .route("/api/workspace/create", post(api_workspace_create))
        .route("/api/workspace/join", post(api_workspace_join))
        .route(
            "/api/workspace/:id/agents",
            get(api_workspace_agents_get).post(api_workspace_agents_post),
        )
        .route("/api/workspace/:id/status", get(api_workspace_status))
        .route("/api/workspace/:id/feed", get(api_workspace_feed))
        .route("/api/workspace/:id/message", post(api_workspace_message));
    let router = crate::serve_core::servecore_apply_pipeline(router);
    let router = crate::serve_core::servecore_with_shared_state(router, serve_core_state);
    router.fallback(api_not_found).with_state(state)
}

async fn api_send(
    State(state): State<Arc<ServeState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    if let Some(response) = verify_protected_request(&state, peer, &method, &uri, &headers, &body) {
        response
    } else {
        let parsed = serde_json::from_slice::<SendBody>(&body).unwrap_or_default();
        Json(json!({
            "ok": true,
            "target": parsed.target.unwrap_or_else(|| "unknown".to_owned()),
            "text": parsed.text.unwrap_or_default(),
            "source": "maw-rs",
            "state": "queued"
        }))
        .into_response()
    }
}

async fn api_feed_get() -> impl IntoResponse {
    Json(json!({"events": [], "total": 0, "active_oracles": []}))
}

async fn api_feed_post(
    State(state): State<Arc<ServeState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    if let Some(response) = verify_protected_request(&state, peer, &method, &uri, &headers, &body) {
        response
    } else {
        Json(json!({"ok": true})).into_response()
    }
}

async fn api_sessions(Query(query): Query<SessionsQuery>) -> impl IntoResponse {
    let _ = query.local.unwrap_or(false);
    Json(Vec::<Value>::new())
}

async fn api_capture(Query(query): Query<CaptureQuery>) -> impl IntoResponse {
    Json(json!({"content": "", "target": query.target}))
}

async fn api_probe(
    State(state): State<Arc<ServeState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    if let Some(response) = verify_protected_request(&state, peer, &method, &uri, &headers, &body) {
        response
    } else {
        Json(json!({"ok": true, "transport": "local", "source": "maw-rs", "sessions": []})).into_response()
    }
}

async fn api_wake(
    State(state): State<Arc<ServeState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    if let Some(response) = verify_protected_request(&state, peer, &method, &uri, &headers, &body) {
        response
    } else {
        Json(json!({"ok": true})).into_response()
    }
}

async fn api_pane_keys(
    State(state): State<Arc<ServeState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    if let Some(response) = verify_protected_request(&state, peer, &method, &uri, &headers, &body) {
        response
    } else {
        Json(json!({"ok": true})).into_response()
    }
}

async fn api_transport_status() -> impl IntoResponse {
    Json(json!({"transports": [{"name": "http-federation", "connected": true}]}))
}

async fn api_transport_send(
    State(state): State<Arc<ServeState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    if let Some(response) = verify_protected_request(&state, peer, &method, &uri, &headers, &body) {
        response
    } else {
        (
            StatusCode::BAD_GATEWAY,
            Json(json!({"ok": false, "via": "http", "reason": "peer-forward-unavailable", "retryable": true})),
        )
            .into_response()
    }
}

async fn api_health() -> impl IntoResponse {
    Json(json!({"ok": true, "source": "maw-rs", "server": "local", "port": DEFAULT_SERVE_PORT}))
}

async fn api_message_ledger(Query(query): Query<MessageLedgerQuery>) -> impl IntoResponse {
    let _ = (query.limit, query.from, query.to, query.direction, query.state, query.q, query.json);
    Json(json!({"ok": true, "messages": [], "total": 0, "source": "maw-rs-native"}))
}

async fn api_requests(
    State(state): State<Arc<ServeState>>,
    Query(query): Query<RequestListQuery>,
) -> impl IntoResponse {
    let requests = with_request_store(&state, |store| store.list(query.oracle.as_deref(), query.status.as_deref()));
    Json(json!({"requests": requests, "total": requests.len()}))
}

async fn api_request_create(
    State(state): State<Arc<ServeState>>,
    Json(body): Json<RequestCreateBody>,
) -> impl IntoResponse {
    let entry = with_request_store(&state, |store| store.create(body));
    Json(json!({"correlationId": entry.correlation_id, "status": entry.status, "oracle": entry.to}))
}

async fn api_reply(
    State(state): State<Arc<ServeState>>,
    AxumPath(correlation_id): AxumPath<String>,
    Json(body): Json<ReplyBody>,
) -> impl IntoResponse {
    with_request_store(&state, |store| match store.reply(&correlation_id, body.reply, body.data) {
        ReplyResult::Ok => Json(json!({"ok": true, "correlationId": correlation_id})).into_response(),
        ReplyResult::NotFound => (StatusCode::NOT_FOUND, Json(json!({"error": "request not found"}))).into_response(),
        ReplyResult::AlreadyReplied => Json(json!({"error": "already replied", "correlationId": correlation_id})).into_response(),
    })
}


async fn api_trust_list(State(state): State<Arc<ServeState>>) -> impl IntoResponse {
    match trust_read_store(&state.trust_store_path) {
        Ok(entries) => Json(json!({
            "ok": true,
            "entries": trust_entries_json(&entries),
            "total": entries.len()
        }))
        .into_response(),
        Err(message) => trust_http_error(StatusCode::INTERNAL_SERVER_ERROR, &message),
    }
}

async fn api_trust_add(
    State(state): State<Arc<ServeState>>,
    Json(body): Json<TrustAddBody>,
) -> impl IntoResponse {
    match trust_store_add(
        &state.trust_store_path,
        &body.sender,
        &body.target,
        &body.peer_key,
        unix_millis_i64(),
    ) {
        Ok(outcome) => Json(json!({
            "ok": true,
            "state": trust_outcome_state(&outcome),
            "sender": body.sender,
            "target": body.target,
            "peerKey": "received (redacted)"
        }))
        .into_response(),
        Err(message) => trust_http_error(StatusCode::BAD_REQUEST, &message),
    }
}

async fn api_trust_revoke(
    State(state): State<Arc<ServeState>>,
    Json(body): Json<TrustRevokeBody>,
) -> impl IntoResponse {
    if !body.yes.unwrap_or(false) {
        return trust_http_error(StatusCode::BAD_REQUEST, "trust revoke: missing explicit yes");
    }
    match trust_store_remove(&state.trust_store_path, &body.sender, &body.target) {
        Ok(true) => Json(json!({"ok": true, "state": "revoked"})).into_response(),
        Ok(false) => trust_http_error(StatusCode::NOT_FOUND, "trust revoke: entry not found"),
        Err(message) => trust_http_error(StatusCode::BAD_REQUEST, &message),
    }
}

fn trust_entries_json(entries: &[TrustEntryPlan]) -> Vec<Value> {
    let mut rows = entries.to_vec();
    rows.sort_by(|left, right| left.added_at.cmp(&right.added_at));
    rows.into_iter()
        .map(|entry| {
            json!({
                "sender": entry.sender,
                "target": entry.target,
                "addedAt": entry.added_at,
                "peerKey": if entry.peer_key.is_some() { "received (redacted)" } else { "missing" }
            })
        })
        .collect()
}

fn trust_outcome_state(outcome: &TrustWriteOutcome) -> &'static str {
    match outcome {
        TrustWriteOutcome::Added => "trusted",
        TrustWriteOutcome::AlreadyTrusted => "already-trusted",
        TrustWriteOutcome::UpdatedPin => "pin-updated",
    }
}

fn trust_http_error(status: StatusCode, message: &str) -> axum::response::Response {
    (status, Json(json!({"ok": false, "error": message}))).into_response()
}

fn unix_millis_i64() -> i64 {
    i64::try_from(unix_millis()).unwrap_or(i64::MAX)
}

async fn api_workspace_create(
    State(state): State<Arc<ServeState>>,
    Json(body): Json<WorkspaceCreateBody>,
) -> impl IntoResponse {
    let workspace = Workspace::new(body.name, body.node_id);
    let response = json!({
        "id": workspace.id,
        "token": workspace.token,
        "joinCode": workspace.join_code,
        "joinCodeExpiresAt": workspace.join_code_expires_at,
    });
    with_workspace_store(&state, |store| {
        store.join_codes.insert(workspace.join_code.clone(), workspace.id.clone());
        store.workspaces.insert(workspace.id.clone(), workspace);
    });
    Json(response).into_response()
}

async fn api_workspace_join(
    State(state): State<Arc<ServeState>>,
    Json(body): Json<WorkspaceJoinBody>,
) -> impl IntoResponse {
    with_workspace_store(&state, |store| {
        let Some(workspace_id) = store.join_codes.get(&body.code).cloned() else {
            return (StatusCode::NOT_FOUND, Json(json!({"error": "not_found"}))).into_response();
        };
        let Some(workspace) = store.workspaces.get_mut(&workspace_id) else {
            return (StatusCode::NOT_FOUND, Json(json!({"error": "not_found"}))).into_response();
        };
        workspace.nodes.insert(body.node_id);
        Json(json!({
            "workspaceId": workspace.id,
            "token": workspace.token,
            "name": workspace.name,
        }))
        .into_response()
    })
}

async fn api_workspace_agents_post(
    State(state): State<Arc<ServeState>>,
    AxumPath(id): AxumPath<String>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    if let Some(response) = verify_workspace_request(&state, &id, &method, &uri, &headers) {
        return response;
    }
    let agent = serde_json::from_slice::<WorkspaceAgentBody>(&body).unwrap_or_default();
    with_workspace_store(&state, |store| {
        let Some(workspace) = store.workspaces.get_mut(&id) else {
            return workspace_not_found();
        };
        if !agent.node_id.is_empty() {
            workspace.nodes.insert(agent.node_id.clone());
        }
        if !agent.name.is_empty() {
            workspace.agents.insert(
                agent_key(&agent.node_id, &agent.name),
                WorkspaceAgent {
                    name: agent.name,
                    node_id: agent.node_id,
                    status: agent.status,
                    capabilities: agent.capabilities,
                },
            );
        }
        Json(json!({"ok": true, "agents": workspace.agents.len()})).into_response()
    })
}

async fn api_workspace_agents_get(
    State(state): State<Arc<ServeState>>,
    AxumPath(id): AxumPath<String>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Some(response) = verify_workspace_request(&state, &id, &method, &uri, &headers) {
        return response;
    }
    with_workspace_store(&state, |store| {
        let Some(workspace) = store.workspaces.get(&id) else {
            return workspace_not_found();
        };
        let agents = workspace
            .agents
            .values()
            .map(|agent| {
                json!({
                    "name": agent.name,
                    "nodeId": agent.node_id,
                    "status": agent.status,
                    "capabilities": agent.capabilities,
                })
            })
            .collect::<Vec<_>>();
        Json(json!({"agents": agents, "total": workspace.agents.len()})).into_response()
    })
}

async fn api_workspace_status(
    State(state): State<Arc<ServeState>>,
    AxumPath(id): AxumPath<String>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Some(response) = verify_workspace_request(&state, &id, &method, &uri, &headers) {
        return response;
    }
    with_workspace_store(&state, |store| {
        let Some(workspace) = store.workspaces.get(&id) else {
            return workspace_not_found();
        };
        Json(json!({
            "id": workspace.id,
            "name": workspace.name,
            "createdAt": workspace.created_at,
            "nodes": workspace.nodes.iter().cloned().collect::<Vec<_>>(),
            "nodeCount": workspace.nodes.len(),
            "healthyNodes": workspace.nodes.len(),
            "agents": workspace.agents.values().map(|agent| json!({"name": agent.name, "nodeId": agent.node_id, "status": agent.status, "capabilities": agent.capabilities})).collect::<Vec<_>>(),
            "agentCount": workspace.agents.len(),
            "feedCount": workspace.feed.len(),
        }))
        .into_response()
    })
}

async fn api_workspace_feed(
    State(state): State<Arc<ServeState>>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<WorkspaceFeedQuery>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Some(response) = verify_workspace_request(&state, &id, &method, &uri, &headers) {
        return response;
    }
    with_workspace_store(&state, |store| {
        let Some(workspace) = store.workspaces.get(&id) else {
            return workspace_not_found();
        };
        let limit = query.limit.unwrap_or(workspace.feed.len());
        let start = workspace.feed.len().saturating_sub(limit);
        Json(json!({"events": workspace.feed[start..].to_vec(), "total": workspace.feed.len()}))
            .into_response()
    })
}

async fn api_workspace_message(
    State(state): State<Arc<ServeState>>,
    AxumPath(id): AxumPath<String>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    if let Some(response) = verify_workspace_request(&state, &id, &method, &uri, &headers) {
        return response;
    }
    let message = serde_json::from_slice::<WorkspaceMessageBody>(&body).unwrap_or_default();
    with_workspace_store(&state, |store| {
        let Some(workspace) = store.workspaces.get_mut(&id) else {
            return workspace_not_found();
        };
        workspace.feed.push(json!({
            "from": message.from,
            "text": message.text,
            "to": message.to,
            "timestamp": unix_seconds(),
        }));
        Json(json!({"ok": true})).into_response()
    })
}

async fn api_not_found() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, Json(json!({"error": "not_found"})))
}

fn verify_protected_request(
    state: &ServeState,
    peer: SocketAddr,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: &Bytes,
) -> Option<axum::response::Response> {
    let effective_peer = effective_peer_addr(state, peer);
    if maw_auth::is_loopback(Some(&effective_peer.ip().to_string())) {
        return None;
    }
    let now = verify_now(state);
    let decision = verify_request(&VerifyRequestArgs {
        method: method.as_str().to_owned(),
        path: path_and_query(uri),
        headers: extract_auth_headers(headers),
        body: Some(body.to_vec()),
        cached_pubkey: state.cached_pubkey.clone(),
        now,
    });
    if maw_auth::is_refuse_decision(&decision) {
        return Some((
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unauthorized", "decision": decision.kind()})),
        )
            .into_response());
    }
    None
}

#[cfg(test)]
fn effective_peer_addr(state: &ServeState, peer: SocketAddr) -> SocketAddr {
    state.peer_addr_override.unwrap_or(peer)
}

#[cfg(not(test))]
fn effective_peer_addr(_state: &ServeState, peer: SocketAddr) -> SocketAddr {
    peer
}

#[cfg(test)]
fn verify_now(state: &ServeState) -> i64 {
    state
        .now_override
        .unwrap_or_else(|| i64::try_from(current_epoch_seconds()).unwrap_or(i64::MAX))
}

#[cfg(not(test))]
fn verify_now(_state: &ServeState) -> i64 {
    i64::try_from(current_epoch_seconds()).unwrap_or(i64::MAX)
}

fn extract_auth_headers(headers: &HeaderMap) -> Headers {
    Headers::new([
        ("x-maw-from", header_to_string(headers, "x-maw-from")),
        (
            "x-maw-signature-v3",
            header_to_string(headers, "x-maw-signature-v3"),
        ),
        (
            "x-maw-timestamp",
            header_to_string(headers, "x-maw-timestamp"),
        ),
        (
            "x-maw-signed-at",
            header_to_string(headers, "x-maw-signed-at"),
        ),
        (
            "x-maw-signature",
            header_to_string(headers, "x-maw-signature"),
        ),
        (
            "x-maw-auth-version",
            header_to_string(headers, "x-maw-auth-version"),
        ),
    ])
}

fn header_to_string(headers: &HeaderMap, name: &str) -> String {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

fn path_and_query(uri: &Uri) -> String {
    uri.path_and_query()
        .map_or_else(|| uri.path().to_owned(), ToString::to_string)
}

fn verify_workspace_request(
    state: &ServeState,
    id: &str,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
) -> Option<axum::response::Response> {
    with_workspace_store(state, |store| {
        let Some(workspace) = store.workspaces.get(id) else {
            return Some(workspace_not_found());
        };
        let timestamp = header_to_string(headers, "x-maw-timestamp");
        let signature = header_to_string(headers, "x-maw-signature");
        let Some(signed_at) = parse_workspace_timestamp(&timestamp) else {
            return Some(workspace_auth_failed());
        };
        let now = verify_now(state);
        if (now - signed_at).abs() > 300 {
            return Some(workspace_auth_failed());
        }
        let payload = format!("{}:{}:{}", method.as_str(), uri.path(), timestamp);
        if maw_auth::verify_hmac_sig(&workspace.token, &payload, &signature) {
            None
        } else {
            Some(workspace_auth_failed())
        }
    })
}

fn parse_workspace_timestamp(timestamp: &str) -> Option<i64> {
    if timestamp.chars().all(|ch| ch.is_ascii_digit()) {
        timestamp.parse().ok()
    } else {
        None
    }
}

fn workspace_auth_failed() -> axum::response::Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({"error": "unauthorized"})),
    )
        .into_response()
}

fn workspace_not_found() -> axum::response::Response {
    (StatusCode::NOT_FOUND, Json(json!({"error": "not_found"}))).into_response()
}

fn with_workspace_store<T>(state: &ServeState, op: impl FnOnce(&mut WorkspaceStore) -> T) -> T {
    let mut guard = state
        .workspaces
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    op(&mut guard)
}

fn random_hex(bytes: usize) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut data = vec![0_u8; bytes];
    rand::thread_rng().fill_bytes(&mut data);
    let mut output = String::with_capacity(bytes * 2);
    for byte in data {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn unix_seconds() -> i64 {
    i64::try_from(current_epoch_seconds()).unwrap_or(i64::MAX)
}

fn unix_millis() -> u64 {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
}

fn agent_key(node_id: &str, name: &str) -> String {
    format!("{node_id}:{name}")
}

fn load_inbound_peer_pubkey() -> Option<String> {
    if let Ok(value) = std::env::var("MAW_PEER_PUBKEY") {
        if !value.trim().is_empty() {
            return Some(value.trim().to_owned());
        }
    }
    let env = real_xdg_env();
    let paths = [
        maw_state_path(&env, &["peers.json"]),
        maw_config_path(&env, &["maw.config.json"]),
    ];
    paths
        .iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .filter_map(|raw| serde_json::from_str::<Value>(&raw).ok())
        .find_map(|value| find_first_pubkey(&value))
}

fn find_first_pubkey(value: &Value) -> Option<String> {
    match value {
        Value::Object(map) => {
            for key in ["pubkey", "pubKey", "peerKey", "publicKey"] {
                if let Some(found) = map
                    .get(key)
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                {
                    return Some(found.to_owned());
                }
            }
            map.values().find_map(find_first_pubkey)
        }
        Value::Array(items) => items.iter().find_map(find_first_pubkey),
        _ => None,
    }
}

#[derive(Default, Deserialize)]
struct SendBody {
    target: Option<String>,
    text: Option<String>,
}

#[derive(Default)]
struct RequestReplyStore {
    entries: HashMap<String, RequestEntry>,
    next_id: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RequestEntry {
    correlation_id: String,
    from: String,
    to: String,
    target: String,
    message: String,
    status: String,
    reply: Option<String>,
    data: Option<Value>,
}

enum ReplyResult {
    Ok,
    NotFound,
    AlreadyReplied,
}

impl RequestReplyStore {
    fn create(&mut self, body: RequestCreateBody) -> RequestEntry {
        self.next_id = self.next_id.saturating_add(1);
        let correlation_id = format!("req-{}", self.next_id);
        let to = body.to.split(':').next().unwrap_or(&body.to).to_owned();
        let entry = RequestEntry {
            correlation_id: correlation_id.clone(),
            from: body.from.unwrap_or_else(|| "external".to_owned()),
            to,
            target: body.to,
            message: body.message,
            status: "delivered".to_owned(),
            reply: None,
            data: None,
        };
        self.entries.insert(correlation_id, entry.clone());
        entry
    }

    fn list(&self, oracle: Option<&str>, status: Option<&str>) -> Vec<RequestEntry> {
        let mut entries = self.entries.values().cloned().collect::<Vec<_>>();
        entries.sort_by(|a, b| a.correlation_id.cmp(&b.correlation_id));
        entries
            .into_iter()
            .filter(|entry| oracle.is_none_or(|oracle| entry.to == oracle))
            .filter(|entry| status.is_none_or(|status| entry.status == status))
            .collect()
    }

    fn reply(&mut self, correlation_id: &str, reply: String, data: Option<Value>) -> ReplyResult {
        let Some(entry) = self.entries.get_mut(correlation_id) else {
            return ReplyResult::NotFound;
        };
        if entry.status == "replied" {
            return ReplyResult::AlreadyReplied;
        }
        "replied".clone_into(&mut entry.status);
        entry.reply = Some(reply);
        entry.data = data;
        ReplyResult::Ok
    }
}

fn with_request_store<T>(state: &ServeState, f: impl FnOnce(&mut RequestReplyStore) -> T) -> T {
    match state.requests.lock() {
        Ok(mut store) => f(&mut store),
        Err(poisoned) => {
            let mut store = poisoned.into_inner();
            f(&mut store)
        }
    }
}

#[derive(Deserialize)]
struct MessageLedgerQuery {
    limit: Option<usize>,
    from: Option<String>,
    to: Option<String>,
    direction: Option<String>,
    state: Option<String>,
    q: Option<String>,
    json: Option<String>,
}

#[derive(Deserialize)]
struct RequestListQuery {
    oracle: Option<String>,
    status: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrustAddBody {
    sender: String,
    target: String,
    peer_key: String,
}

#[derive(Deserialize)]
struct TrustRevokeBody {
    sender: String,
    target: String,
    yes: Option<bool>,
}

#[derive(Default, Deserialize)]
struct RequestCreateBody {
    to: String,
    message: String,
    from: Option<String>,
}

#[derive(Deserialize)]
struct ReplyBody {
    reply: String,
    data: Option<Value>,
}

#[derive(Default)]
struct WorkspaceStore {
    workspaces: HashMap<String, Workspace>,
    join_codes: HashMap<String, String>,
}

struct Workspace {
    id: String,
    name: String,
    token: String,
    join_code: String,
    join_code_expires_at: u64,
    created_at: u64,
    nodes: HashSet<String>,
    agents: HashMap<String, WorkspaceAgent>,
    feed: Vec<Value>,
}

impl Workspace {
    fn new(name: String, node_id: String) -> Self {
        let created_at = unix_millis();
        let mut nodes = HashSet::new();
        nodes.insert(node_id);
        Self {
            id: format!("ws-{}", random_hex(8)),
            name,
            token: random_hex(32),
            join_code: random_hex(3),
            join_code_expires_at: created_at.saturating_add(15 * 60 * 1_000),
            created_at,
            nodes,
            agents: HashMap::new(),
            feed: Vec::new(),
        }
    }
}

struct WorkspaceAgent {
    name: String,
    node_id: String,
    status: Option<String>,
    capabilities: Option<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceCreateBody {
    name: String,
    node_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceJoinBody {
    code: String,
    node_id: String,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceAgentBody {
    name: String,
    node_id: String,
    status: Option<String>,
    capabilities: Option<Value>,
}

#[derive(Default, Deserialize)]
struct WorkspaceMessageBody {
    from: String,
    text: String,
    to: Option<String>,
}

#[derive(Deserialize)]
struct WorkspaceFeedQuery {
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct SessionsQuery {
    local: Option<bool>,
}

#[derive(Deserialize)]
struct CaptureQuery {
    target: Option<String>,
}

#[cfg(test)]
#[allow(clippy::redundant_closure_for_method_calls)]
mod serve_tests {
    use super::*;
    use axum::body::Body;
    use futures_util::{SinkExt, StreamExt};
    use maw_auth::{build_legacy_from_sign_payload, hash_body, sign_headers_v3_at, sign_hmac_sig};
    use tokio::sync::oneshot;
    use tower::ServiceExt;

    const KEY: &str = "test-peer-key-0123456789";
    const FROM: &str = "sender-oracle:sender-node";


    fn serve_test_trust_store_path(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "maw-rs-trust-live-{label}-{}-{}.json",
            std::process::id(),
            random_hex(4)
        ))
    }

    fn serve_test_app(trust_store_path: std::path::PathBuf) -> Router {
        serve_router(ServeState {
            cached_pubkey: Some(KEY.to_owned()),
            workspaces: Mutex::new(WorkspaceStore::default()),
            requests: Mutex::new(RequestReplyStore::default()),
            peer_addr_override: Some(NON_LOOPBACK_TEST_PEER),
            now_override: Some(1_782_277_200),
            serve_core_state_override: None,
            trust_store_path,
        })
    }

    fn signed_trust_request(method: &str, uri: &str, auth_path: &str, body: &'static str) -> axum::http::Request<Body> {
        let headers = sign_headers_v3_at(
            KEY,
            FROM,
            method,
            auth_path,
            Some(body.as_bytes()),
            1_782_277_200,
        )
        .expect("sign trust");
        let mut builder = axum::http::Request::builder()
            .method(method)
            .uri(uri)
            .header(reqwest::header::CONTENT_TYPE, "application/json");
        for (name, value) in headers.to_btree_map() {
            builder = builder.header(name, value);
        }
        let mut request = builder.body(Body::from(body)).expect("request");
        request.extensions_mut().insert(ConnectInfo(NON_LOOPBACK_TEST_PEER));
        request
    }

    fn unsigned_trust_request(method: &str, uri: &str, body: &'static str) -> axum::http::Request<Body> {
        let mut request = axum::http::Request::builder()
            .method(method)
            .uri(uri)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(Body::from(body))
            .expect("request");
        request.extensions_mut().insert(ConnectInfo(NON_LOOPBACK_TEST_PEER));
        request
    }

    async fn response_json(response: axum::response::Response) -> Value {
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("body");
        serde_json::from_slice(&bytes).expect("json")
    }

    async fn spawn_test_server() -> SocketAddr {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind test server");
        let addr = listener.local_addr().expect("local addr");
        let app = serve_router(ServeState {
            cached_pubkey: Some(KEY.to_owned()),
            workspaces: Mutex::new(WorkspaceStore::default()),
            requests: Mutex::new(RequestReplyStore::default()),
            peer_addr_override: Some(NON_LOOPBACK_TEST_PEER),
            now_override: Some(1_782_277_200),
            serve_core_state_override: None,
            trust_store_path: serve_test_trust_store_path("server"),
        });
        let (tx, rx) = oneshot::channel::<()>();
        tokio::spawn(async move {
            let server = axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(async move {
                let _ = rx.await;
            });
            server.await.expect("serve test server");
        });
        std::mem::forget(tx);
        addr
    }

    #[tokio::test]
    async fn serve_real_wire_accepts_v3_rejects_unsigned_and_accepts_legacy() {
        let addr = spawn_test_server().await;
        let client = reqwest::Client::builder().build().expect("client");
        let url = format!("http://{addr}/api/send");
        let body = r#"{"target":"remote-oracle","text":"hello","inbox":true}"#;
        let timestamp = 1_782_277_200_i64;
        let headers = sign_headers_v3_at(
            KEY,
            FROM,
            "POST",
            "/api/send",
            Some(body.as_bytes()),
            timestamp,
        )
        .expect("sign v3");
        let mut request = client
            .post(&url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.to_owned());
        for (name, value) in headers.to_btree_map() {
            request = request.header(name, value);
        }
        let response = request.send().await.expect("send signed");
        assert_eq!(response.status(), StatusCode::OK);
        let payload = response.json::<Value>().await.expect("json");
        assert_eq!(payload["ok"], true);
        assert_eq!(payload["state"], "queued");

        let response = client
            .post(&url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header("x-forwarded-for", "127.0.0.1")
            .body(body.to_owned())
            .send()
            .await
            .expect("send unsigned");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let signed_at = "2026-06-24T05:00:00.000Z";
        let now = 1_782_277_200_i64;
        let body_hash = hash_body(Some(body.as_bytes()));
        let payload = build_legacy_from_sign_payload(FROM, signed_at, "POST", "/api/send", &body_hash);
        let legacy_sig = sign_hmac_sig(KEY, &payload);
        let response = client
            .post(&url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header("x-maw-from", FROM)
            .header("x-maw-signature", legacy_sig)
            .header("x-maw-signed-at", signed_at)
            .header("x-maw-auth-version", "v3")
            .header("x-maw-timestamp", now.to_string())
            .body(body.to_owned())
            .send()
            .await
            .expect("send legacy");
        assert_eq!(response.status(), StatusCode::OK);
    }


    #[tokio::test]
    async fn serve_trust_live_is_auth_gated_atomic_redacted_and_tofu_safe() {
        let path = serve_test_trust_store_path("route");
        let app = serve_test_app(path.clone());
        assert!(maw_auth::is_protected("/api/trust", "POST"));
        assert!(maw_auth::is_protected("/api/trust/revoke", "POST"));
        assert!(maw_auth::is_protected("/api/trust", "GET"));

        let secret_key = "ed25519:alpha-peer-key-secret";
        let body = r#"{"sender":"alpha","target":"beta","peerKey":"ed25519:alpha-peer-key-secret"}"#;
        let denied = app
            .clone()
            .oneshot(unsigned_trust_request("POST", "/api/trust", body))
            .await
            .expect("denied");
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);

        let trusted = app
            .clone()
            .oneshot(signed_trust_request("POST", "/api/trust", "/trust", body))
            .await
            .expect("trust");
        let trusted_status = trusted.status();
        let payload = response_json(trusted).await;
        assert_eq!(trusted_status, StatusCode::OK, "{payload}");
        let rendered = payload.to_string();
        assert_eq!(payload["peerKey"], "received (redacted)");
        assert!(!rendered.contains(secret_key), "{rendered}");
        let stored = std::fs::read_to_string(&path).expect("stored");
        assert!(stored.contains(secret_key));
        assert!(!path.with_extension("json.tmp").exists());

        let mismatch = r#"{"sender":"beta","target":"alpha","peerKey":"ed25519:different-peer-key"}"#;
        let rejected = app
            .clone()
            .oneshot(signed_trust_request("POST", "/api/trust", "/trust", mismatch))
            .await
            .expect("mismatch");
        assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
        let rejected_payload = response_json(rejected).await.to_string();
        assert!(rejected_payload.contains("peer-key mismatch"));
        assert!(!rejected_payload.contains("different-peer-key"));

        let listed = app
            .clone()
            .oneshot(signed_trust_request("GET", "/api/trust", "/trust", ""))
            .await
            .expect("list");
        assert_eq!(listed.status(), StatusCode::OK);
        let listed_payload = response_json(listed).await.to_string();
        assert!(listed_payload.contains("received (redacted)"));
        assert!(!listed_payload.contains(secret_key));

        let missing_yes = r#"{"sender":"alpha","target":"beta"}"#;
        let refused = app
            .clone()
            .oneshot(signed_trust_request(
                "POST",
                "/api/trust/revoke",
                "/trust/revoke",
                missing_yes,
            ))
            .await
            .expect("missing yes");
        assert_eq!(refused.status(), StatusCode::BAD_REQUEST);

        let revoke = r#"{"sender":"alpha","target":"beta","yes":true}"#;
        let revoked = app
            .oneshot(signed_trust_request(
                "POST",
                "/api/trust/revoke",
                "/trust/revoke",
                revoke,
            ))
            .await
            .expect("revoke");
        assert_eq!(revoked.status(), StatusCode::OK);
        let entries = trust_read_store(&path).expect("read after revoke");
        assert!(entries.is_empty());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn serve_default_bind_is_loopback_only() {
        let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        let _restore = EnvVarRestore::capture("MAW_HOST");
        std::env::set_var("MAW_HOST", "0.0.0.0");
        assert_eq!(default_bind_host(), "127.0.0.1");
    }

    #[tokio::test]
    async fn serve_core_real_router_allows_loopback_protected_paths() {
        let addr = spawn_test_server().await;
        let client = reqwest::Client::builder().build().expect("client");
        let trigger = client
            .post(format!("http://{addr}/api/triggers/fire"))
            .json(&json!({"event":"agent-idle","context":{"repo":"maw-rs"}}))
            .send()
            .await
            .expect("protected request");
        assert_eq!(trigger.status(), StatusCode::OK, "/api/triggers/fire");
        let plugins = client
            .post(format!("http://{addr}/api/plugins/reload"))
            .send()
            .await
            .expect("protected request");
        assert_eq!(plugins.status(), StatusCode::OK, "/api/plugins/reload");
        let cleanup = client
            .post(format!("http://{addr}/api/worktrees/cleanup"))
            .send()
            .await
            .expect("protected request");
        assert_eq!(
            cleanup.status(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "/api/worktrees/cleanup is live JSON route, not core stub"
        );
        let public = client
            .get(format!("http://{addr}/api/agents"))
            .send()
            .await
            .expect("public request");
        assert_eq!(public.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn serve_agents_real_router_is_public_and_uses_fake_state() {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind test server");
        let addr = listener.local_addr().expect("local addr");
        let fake_core = crate::serve_core::ServecoreSharedState::default()
            .servecore_with_agents_node(Some("node-a".to_owned()))
            .servecore_with_agents_snapshot(vec![crate::serve_core::ServecoreAgentPane {
                id: "%86".to_owned(),
                command: "codex".to_owned(),
                target: "nova:1.0".to_owned(),
                title: "nova-agent".to_owned(),
                cwd: Some("/tmp/maw-rs".to_owned()),
                pid: Some(8600),
                last_activity: Some(86),
            }]);
        let app = serve_router(ServeState {
            cached_pubkey: Some(KEY.to_owned()),
            workspaces: Mutex::new(WorkspaceStore::default()),
            requests: Mutex::new(RequestReplyStore::default()),
            peer_addr_override: Some(NON_LOOPBACK_TEST_PEER),
            now_override: Some(1_782_277_200),
            serve_core_state_override: Some(fake_core),
            trust_store_path: serve_test_trust_store_path("agents"),
        });
        let (tx, rx) = oneshot::channel::<()>();
        tokio::spawn(async move {
            let server = axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(async move {
                let _ = rx.await;
            });
            server.await.expect("serve test server");
        });
        std::mem::forget(tx);

        let client = reqwest::Client::builder().build().expect("client");
        let response = client
            .get(format!("http://{addr}/api/agents"))
            .send()
            .await
            .expect("agents");
        assert_eq!(response.status(), StatusCode::OK);
        let payload = response.json::<Value>().await.expect("json");
        assert_eq!(payload["count"], 1);
        assert_eq!(payload["node"], "node-a");
        assert_eq!(payload["agents"][0]["target"], "nova:1.0");

        let protected = client
            .post(format!("http://{addr}/api/triggers/fire"))
            .json(&json!({"event":"agent-idle","context":{"repo":"maw-rs"}}))
            .send()
            .await
            .expect("protected");
        assert_eq!(protected.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn serve_real_wire_websocket_relay_echoes_text_frame() {
        let addr = spawn_test_server().await;
        let url = format!("ws://{addr}/ws");
        let (mut ws, _response) = tokio_tungstenite::connect_async(&url)
            .await
            .expect("connect websocket");

        ws.send(tokio_tungstenite::tungstenite::Message::Text(
            "relay-check".to_owned(),
        ))
        .await
        .expect("send websocket text");

        let received = ws
            .next()
            .await
            .expect("websocket should yield a frame")
            .expect("frame should be ok");
        assert_eq!(
            received,
            tokio_tungstenite::tungstenite::Message::Text("relay-check".to_owned())
        );
    }

    #[tokio::test]
    async fn workspace_hub_signed_routes_accept_and_unsigned_rejects() {
        let addr = spawn_test_server().await;
        let client = reqwest::Client::builder().build().expect("client");
        let create_url = format!("http://{addr}/api/workspace/create");
        let create_response = client
            .post(create_url)
            .json(&json!({"name": "nova", "nodeId": "node-a"}))
            .send()
            .await
            .expect("create workspace");
        assert_eq!(create_response.status(), StatusCode::OK);
        let create_payload = create_response.json::<Value>().await.expect("create json");
        let workspace_id = create_payload["id"].as_str().expect("workspace id");
        let token = create_payload["token"].as_str().expect("workspace token");
        assert_eq!(token.len(), 64);

        let agents_path = format!("/api/workspace/{workspace_id}/agents");
        let agents_url = format!("http://{addr}{agents_path}");
        let unsigned = client
            .post(&agents_url)
            .json(&json!({"name": "nova-codex-1", "nodeId": "node-a"}))
            .send()
            .await
            .expect("unsigned agents request");
        assert_eq!(unsigned.status(), StatusCode::UNAUTHORIZED);

        let timestamp = "1782277200";
        let signature = sign_hmac_sig(token, &format!("POST:{agents_path}:{timestamp}"));
        let signed = client
            .post(&agents_url)
            .header("x-maw-timestamp", timestamp)
            .header("x-maw-signature", signature)
            .json(&json!({
                "name": "nova-codex-1",
                "nodeId": "node-a",
                "status": "online",
                "capabilities": ["relay"]
            }))
            .send()
            .await
            .expect("signed agents request");
        assert_eq!(signed.status(), StatusCode::OK);
        let signed_payload = signed.json::<Value>().await.expect("signed json");
        assert_eq!(signed_payload["ok"], true);
        assert_eq!(signed_payload["agents"], 1);
    }
}
