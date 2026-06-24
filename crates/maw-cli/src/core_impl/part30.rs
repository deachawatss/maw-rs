use axum::{
    body::Bytes,
    extract::{ConnectInfo, Query, State},
    http::{HeaderMap, Method, StatusCode, Uri},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
};

const DEFAULT_SERVE_PORT: u16 = 31745;
const DEFAULT_SERVE_BIND: &str = "127.0.0.1";
#[cfg(test)]
const NON_LOOPBACK_TEST_PEER: SocketAddr =
    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 10)), 49_152);

#[derive(Clone)]
struct ServeState {
    cached_pubkey: Option<String>,
    #[cfg(test)]
    peer_addr_override: Option<SocketAddr>,
    #[cfg(test)]
    now_override: Option<i64>,
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
        #[cfg(test)]
        peer_addr_override: None,
        #[cfg(test)]
        now_override: None,
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

fn serve_router(state: ServeState) -> Router {
    let state = Arc::new(state);
    Router::new()
        .route("/api/send", post(api_send))
        .route("/api/feed", get(api_feed_get).post(api_feed_post))
        .route("/api/sessions", get(api_sessions))
        .route("/api/capture", get(api_capture))
        .route("/api/probe", post(api_probe))
        .route("/api/wake", post(api_wake))
        .route("/api/pane-keys", post(api_pane_keys))
        .route("/api/transport/status", get(api_transport_status))
        .route("/api/transport/send", post(api_transport_send))
        .route("/api/federation/status", get(api_federation_status))
        .route("/api/identity", get(api_identity))
        .route("/api/peers/discoveries", get(api_peers_discoveries))
        .route("/api/peers/discovered", get(api_peers_discoveries))
        .fallback(api_not_found)
        .with_state(state)
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

async fn api_federation_status() -> impl IntoResponse {
    Json(json!({"localUrl": null, "localReachable": true, "peers": [], "totalPeers": 0, "reachablePeers": 0, "clockHealth": "ok"}))
}

async fn api_identity() -> impl IntoResponse {
    let config = load_hey_config();
    Json(json!({"ok": true, "node": config.node, "oracle": config.oracle, "agents": []}))
}

async fn api_peers_discoveries() -> impl IntoResponse {
    Json(json!({"ok": true, "total": 0, "shown": 0, "filtered": 0, "peers": []}))
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

#[derive(Deserialize)]
struct SessionsQuery {
    local: Option<bool>,
}

#[derive(Deserialize)]
struct CaptureQuery {
    target: Option<String>,
}

#[cfg(test)]
mod serve_tests {
    use super::*;
    use maw_auth::{build_legacy_from_sign_payload, hash_body, sign_headers_v3_at, sign_hmac_sig};
    use tokio::sync::oneshot;

    const KEY: &str = "test-peer-key-0123456789";
    const FROM: &str = "sender-oracle:sender-node";

    async fn spawn_test_server() -> SocketAddr {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind test server");
        let addr = listener.local_addr().expect("local addr");
        let app = serve_router(ServeState {
            cached_pubkey: Some(KEY.to_owned()),
            peer_addr_override: Some(NON_LOOPBACK_TEST_PEER),
            now_override: Some(1_782_277_200),
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

    #[test]
    fn serve_default_bind_is_loopback_only() {
        std::env::set_var("MAW_HOST", "0.0.0.0");
        assert_eq!(default_bind_host(), "127.0.0.1");
        std::env::remove_var("MAW_HOST");
    }
}
