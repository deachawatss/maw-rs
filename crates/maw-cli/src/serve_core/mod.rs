pub mod modules;

use axum::{
    body::Body,
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    http::{Method, Request, StatusCode, Uri},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{any, get, post},
    Extension, Json, Router,
};
use maw_hub::WorkspaceConfig;
use maw_tmux::{TmuxClient, TmuxPane};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

const SERVECORE_PIPELINE_ORDER: &[&str] = &[
    "cors-preflight",
    "ws-upgrade",
    "engine-proxy",
    "api-protected-auth",
    "registry",
    "api-public",
    "registry",
    "fallback-views",
];
static SERVECORE_WS_CONNECTIONS: AtomicUsize = AtomicUsize::new(0);

pub trait ServecoreEngine: Send + Sync {
    fn servecore_engine_name(&self) -> &'static str;

    /// Opens a websocket stream for a registered serve-core route.
    ///
    /// # Errors
    ///
    /// Implementations may return an error when the requested stream target is unavailable.
    fn servecore_ws_open(
        &self,
        _kind: ServecoreWsKind,
        _target: Option<&str>,
    ) -> Result<(), String> {
        Ok(())
    }

    fn servecore_ws_text(
        &self,
        _kind: ServecoreWsKind,
        text: &str,
        _target: Option<&str>,
    ) -> Option<String> {
        Some(text.to_owned())
    }

    fn servecore_ws_binary(
        &self,
        _kind: ServecoreWsKind,
        bytes: &[u8],
        _target: Option<&str>,
    ) -> Option<Vec<u8>> {
        Some(bytes.to_vec())
    }

    fn servecore_ws_close(&self, _kind: ServecoreWsKind, _target: Option<&str>) {}
}

#[derive(Debug)]
pub struct ServecoreStubEngine;

impl ServecoreEngine for ServecoreStubEngine {
    fn servecore_engine_name(&self) -> &'static str {
        "stub"
    }
}

#[derive(Clone)]
pub struct ServecoreSharedState {
    pub engine: Arc<dyn ServecoreEngine>,
    pub trigger_bus: ServecoreTriggerBus,
    pub lifecycle: ServecoreLifecycle,
    pub hub_workspaces: Arc<Vec<WorkspaceConfig>>,
    pub agents_node: Option<String>,
    pub agents_snapshot: Option<Arc<Vec<ServecoreAgentPane>>>,
}

impl Default for ServecoreSharedState {
    fn default() -> Self {
        Self {
            engine: Arc::new(ServecoreStubEngine),
            trigger_bus: ServecoreTriggerBus::default(),
            lifecycle: ServecoreLifecycle::default(),
            hub_workspaces: Arc::new(Vec::new()),
            agents_node: None,
            agents_snapshot: None,
        }
    }
}

impl ServecoreSharedState {
    #[must_use]
    pub fn servecore_with_engine(mut self, engine: Arc<dyn ServecoreEngine>) -> Self {
        self.engine = engine;
        self
    }

    #[must_use]
    pub fn servecore_with_agents_node(mut self, node: Option<String>) -> Self {
        self.agents_node = node;
        self
    }

    #[must_use]
    pub fn servecore_with_agents_snapshot(mut self, panes: Vec<ServecoreAgentPane>) -> Self {
        self.agents_snapshot = Some(Arc::new(panes));
        self
    }

    #[must_use]
    pub fn servecore_agents_panes(&self) -> Vec<ServecoreAgentPane> {
        if let Some(snapshot) = &self.agents_snapshot {
            return snapshot.as_ref().clone();
        }
        let mut tmux = TmuxClient::local();
        tmux.list_panes()
            .into_iter()
            .map(ServecoreAgentPane::from)
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServecoreAgentPane {
    pub id: String,
    pub command: String,
    pub target: String,
    pub title: String,
    pub cwd: Option<String>,
    pub pid: Option<u32>,
    pub last_activity: Option<u64>,
}

impl From<TmuxPane> for ServecoreAgentPane {
    fn from(pane: TmuxPane) -> Self {
        Self {
            id: pane.id,
            command: pane.command,
            target: pane.target,
            title: pane.title,
            cwd: pane.cwd,
            pid: pane.pid,
            last_activity: pane.last_activity,
        }
    }
}

#[derive(Clone, Default)]
pub struct ServecoreTriggerBus {
    events: Arc<Mutex<VecDeque<ServecoreTriggerEvent>>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServecoreTriggerEvent {
    pub name: String,
    pub payload: String,
}

impl ServecoreTriggerBus {
    pub fn servecore_fire(&self, event: ServecoreTriggerEvent) {
        let mut guard = self
            .events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.push_back(event);
    }

    pub fn servecore_snapshot(&self) -> Vec<ServecoreTriggerEvent> {
        let guard = self
            .events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.iter().cloned().collect()
    }
}

#[derive(Clone, Debug, Default)]
pub struct ServecoreLifecycle {
    modules: Arc<Vec<ServecoreLifecycleModule>>,
    api_routers: Arc<BTreeSet<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServecoreLifecycleModule {
    pub name: String,
    pub weight: i32,
}

impl ServecoreLifecycle {
    #[must_use]
    pub fn servecore_from_profile(
        modules: Vec<ServecoreLifecycleModule>,
        api_routers: &[String],
    ) -> Self {
        let mut sorted = modules;
        sorted.sort_by(|left, right| {
            left.weight
                .cmp(&right.weight)
                .then(left.name.cmp(&right.name))
        });
        Self {
            modules: Arc::new(sorted),
            api_routers: Arc::new(api_routers.iter().cloned().collect()),
        }
    }

    #[must_use]
    pub fn servecore_enabled_modules(&self) -> Vec<String> {
        self.modules
            .iter()
            .filter(|module| self.api_routers.is_empty() || self.api_routers.contains(&module.name))
            .map(|module| module.name.clone())
            .collect()
    }
}

#[derive(Default)]
pub struct ServecoreRouteRegistry {
    seen: BTreeSet<ServecoreRouteKey>,
    routes: Vec<ServecoreRouteKey>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ServecoreRouteKey {
    method: Method,
    path: String,
}

impl ServecoreRouteRegistry {
    /// Register one HTTP route.
    ///
    /// # Errors
    /// Returns an error when the path is invalid or the method/path pair is already registered.
    pub fn servecore_register(&mut self, method: Method, path: &str) -> Result<(), String> {
        servecore_validate_path(path)?;
        let key = ServecoreRouteKey {
            method,
            path: path.to_owned(),
        };
        if !self.seen.insert(key.clone()) {
            return Err(format!(
                "serve-core: duplicate route {} {}",
                key.method, key.path
            ));
        }
        self.routes.push(key);
        Ok(())
    }

    #[must_use]
    pub fn servecore_routes(&self) -> &[ServecoreRouteKey] {
        &self.routes
    }
}

#[derive(Default)]
pub struct ServecoreWsRegistry {
    handlers: BTreeMap<String, ServecoreWsKind>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServecoreWsKind {
    Engine,
    Pty,
    Tmux,
}

impl ServecoreWsRegistry {
    /// Register one websocket upgrade path.
    ///
    /// # Errors
    /// Returns an error when the path is invalid or already registered.
    pub fn servecore_register_ws(&mut self, path: &str) -> Result<(), String> {
        self.servecore_register_ws_kind(path, ServecoreWsKind::Engine)
    }

    /// Register one websocket upgrade path with its stream kind.
    ///
    /// # Errors
    /// Returns an error when the path is invalid or already registered.
    pub fn servecore_register_ws_kind(
        &mut self,
        path: &str,
        kind: ServecoreWsKind,
    ) -> Result<(), String> {
        servecore_validate_path(path)?;
        if self.handlers.insert(path.to_owned(), kind).is_some() {
            return Err(format!("serve-core: duplicate ws route {path}"));
        }
        Ok(())
    }

    #[must_use]
    pub fn servecore_paths(&self) -> Vec<String> {
        self.handlers.keys().cloned().collect()
    }

    #[must_use]
    pub fn servecore_handlers(&self) -> Vec<(String, ServecoreWsKind)> {
        self.handlers
            .iter()
            .map(|(path, kind)| (path.clone(), *kind))
            .collect()
    }
}

pub fn servecore_with_shared_state<S>(router: Router<S>, state: ServecoreSharedState) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    router.layer(Extension(Arc::new(state)))
}

pub fn servecore_mount_core_routes<S>(router: Router<S>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    router
        .route("/api/serve-core/pipeline", get(servecore_pipeline_handler))
        .route("/api/triggers/fire", post(servecore_protected_stub))
        .route("/api/worktrees/cleanup", post(servecore_protected_stub))
        .route("/api/plugins/*plugin_path", post(servecore_protected_stub))
}

pub fn servecore_mount_ws_routes<S>(router: Router<S>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    servecore_mount_ws_routes_with_config(router, modules::ws::WsConfig::ws_from_process_env())
}

pub fn servecore_mount_ws_routes_with_config<S>(
    router: Router<S>,
    config: modules::ws::WsConfig,
) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let registry = servecore_default_ws_registry();
    servecore_mount_ws_registry(router, &registry).layer(Extension(config))
}

pub fn servecore_mount_ws_registry<S>(
    router: Router<S>,
    registry: &ServecoreWsRegistry,
) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    registry
        .servecore_handlers()
        .into_iter()
        .fold(router, |router, (path, kind)| {
            router.route(&path, get(servecore_ws_upgrade).layer(Extension(kind)))
        })
}

fn servecore_default_ws_registry() -> ServecoreWsRegistry {
    let mut registry = ServecoreWsRegistry::default();
    registry
        .servecore_register_ws_kind("/ws", ServecoreWsKind::Engine)
        .expect("default ws route");
    registry
        .servecore_register_ws_kind("/ws/pty", ServecoreWsKind::Pty)
        .expect("default pty ws route");
    registry
        .servecore_register_ws_kind("/ws/tmux", ServecoreWsKind::Tmux)
        .expect("default tmux ws route");
    registry
}

pub fn servecore_mount_registry_stub<S>(
    router: Router<S>,
    registry: &ServecoreRouteRegistry,
) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    registry.routes.iter().fold(router, |router, route| {
        router.route(&route.path, any(servecore_registry_stub))
    })
}

pub fn servecore_apply_pipeline<S>(router: Router<S>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    servecore_apply_pipeline_with_views_config(
        router,
        modules::views::ViewsConfig::views_from_process_env(),
    )
}

pub fn servecore_apply_pipeline_with_views_config<S>(
    router: Router<S>,
    views_config: modules::views::ViewsConfig,
) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    modules::views::views_apply_fallback_with_config(router, views_config)
        .layer(middleware::from_fn(servecore_auth_default_deny))
        .layer(middleware::from_fn(servecore_engine_proxy))
        .layer(middleware::from_fn(servecore_ws_upgrade_gate))
        .layer(middleware::from_fn(servecore_cors_preflight))
}

#[must_use]
pub fn servecore_pipeline_order() -> &'static [&'static str] {
    SERVECORE_PIPELINE_ORDER
}

fn servecore_validate_path(path: &str) -> Result<(), String> {
    if !path.starts_with('/') || path.contains("//") || path.chars().any(char::is_control) {
        return Err("serve-core: route path must be absolute and control-free".to_owned());
    }
    for segment in path.split('/').filter(|segment| !segment.is_empty()) {
        if segment == "--" || segment.starts_with('-') {
            return Err("serve-core: route segment must not start with '-'".to_owned());
        }
    }
    Ok(())
}

async fn servecore_cors_preflight(req: Request<Body>, next: Next) -> Response {
    if req.method() == Method::OPTIONS {
        return StatusCode::NO_CONTENT.into_response();
    }
    next.run(req).await
}

async fn servecore_ws_upgrade_gate(req: Request<Body>, next: Next) -> Response {
    next.run(req).await
}

async fn servecore_engine_proxy(req: Request<Body>, next: Next) -> Response {
    next.run(req).await
}

async fn servecore_auth_default_deny(req: Request<Body>, next: Next) -> Response {
    let method = req.method().clone();
    let path = servecore_api_auth_path(req.uri().path());
    if maw_auth::is_protected(&path, method.as_str()) {
        // TODO(D2): auth logic pending Bigboy+TK; protected paths fail closed meanwhile.
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error":"forbidden","reason":"auth-pending-default-deny"})),
        )
            .into_response();
    }
    next.run(req).await
}

fn servecore_api_auth_path(path: &str) -> String {
    path.strip_prefix("/api").unwrap_or(path).to_owned()
}

async fn servecore_pipeline_handler() -> impl IntoResponse {
    Json(json!({"pipeline": servecore_pipeline_order()}))
}

async fn servecore_protected_stub() -> impl IntoResponse {
    Json(json!({"ok": true, "state": "protected-stub"}))
}

async fn servecore_registry_stub() -> impl IntoResponse {
    Json(json!({"ok": true, "source": "serve-core-registry"}))
}

async fn servecore_ws_upgrade(
    ws: WebSocketUpgrade,
    uri: Uri,
    Extension(kind): Extension<ServecoreWsKind>,
    Extension(state): Extension<Arc<ServecoreSharedState>>,
    Extension(config): Extension<modules::ws::WsConfig>,
) -> impl IntoResponse {
    let target = match modules::ws::ws_validate_target(servecore_ws_target(uri.query())) {
        Ok(target) => target,
        Err(error) => {
            return (StatusCode::BAD_REQUEST, Json(json!({"error":error}))).into_response()
        }
    };
    if state
        .engine
        .servecore_ws_open(kind, target.as_deref())
        .is_err()
    {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error":"ws_engine_unavailable"})),
        )
            .into_response();
    }
    if SERVECORE_WS_CONNECTIONS.load(Ordering::Relaxed) >= config.max_connections {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error":"ws_connection_limit"})),
        )
            .into_response();
    }
    ws.on_upgrade(move |socket| servecore_ws_stream(socket, state, kind, target, config))
        .into_response()
}

async fn servecore_ws_stream(
    mut socket: WebSocket,
    state: Arc<ServecoreSharedState>,
    kind: ServecoreWsKind,
    target: Option<String>,
    config: modules::ws::WsConfig,
) {
    let Some(_guard) = servecore_ws_connection_guard(config.max_connections) else {
        let _ = socket
            .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                code: 1013,
                reason: "ws connection limit".into(),
            })))
            .await;
        return;
    };
    let mut heartbeat = tokio::time::interval_at(
        tokio::time::Instant::now() + config.heartbeat_interval,
        config.heartbeat_interval,
    );
    let idle_timer = tokio::time::sleep(config.idle_timeout);
    tokio::pin!(idle_timer);
    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                if servecore_ws_send(&mut socket, Message::Ping(Vec::new()), config.send_timeout).await.is_err() {
                    break;
                }
            }
            () = &mut idle_timer => {
                let _ = servecore_ws_send(&mut socket, Message::Close(None), config.send_timeout).await;
                break;
            }
            frame = socket.recv() => {
                match frame {
                    Some(Ok(frame)) => {
                        let resets_idle = !matches!(frame, Message::Pong(_));
                        if resets_idle {
                            idle_timer.as_mut().reset(tokio::time::Instant::now() + config.idle_timeout);
                        }
                        if !servecore_ws_handle_frame(&mut socket, &state, kind, target.as_deref(), &config, frame).await {
                            break;
                        }
                    }
                    Some(Err(_)) | None => break,
                }
            }
        }
    }
    state.engine.servecore_ws_close(kind, target.as_deref());
}

async fn servecore_ws_handle_frame(
    socket: &mut WebSocket,
    state: &ServecoreSharedState,
    kind: ServecoreWsKind,
    target: Option<&str>,
    config: &modules::ws::WsConfig,
    frame: Message,
) -> bool {
    match frame {
        Message::Text(text) => {
            if text.len() > config.max_frame_bytes {
                return servecore_ws_send(socket, Message::Close(None), config.send_timeout)
                    .await
                    .is_ok();
            }
            if let Some(reply) = state.engine.servecore_ws_text(kind, &text, target) {
                return servecore_ws_send(socket, Message::Text(reply), config.send_timeout)
                    .await
                    .is_ok();
            }
            true
        }
        Message::Binary(bytes) => {
            if bytes.len() > config.max_frame_bytes {
                return servecore_ws_send(socket, Message::Close(None), config.send_timeout)
                    .await
                    .is_ok();
            }
            if let Some(reply) = state.engine.servecore_ws_binary(kind, &bytes, target) {
                return servecore_ws_send(socket, Message::Binary(reply), config.send_timeout)
                    .await
                    .is_ok();
            }
            true
        }
        Message::Ping(bytes) => {
            servecore_ws_send(socket, Message::Pong(bytes), config.send_timeout)
                .await
                .is_ok()
        }
        Message::Pong(_) => true,
        Message::Close(frame) => {
            let _ = servecore_ws_send(socket, Message::Close(frame), config.send_timeout).await;
            false
        }
    }
}

async fn servecore_ws_send(
    socket: &mut WebSocket,
    message: Message,
    timeout: Duration,
) -> Result<(), ()> {
    tokio::time::timeout(timeout, socket.send(message))
        .await
        .map_err(|_| ())?
        .map_err(|_| ())
}

fn servecore_ws_target(query: Option<&str>) -> Option<&str> {
    query?
        .split('&')
        .filter_map(|part| part.split_once('='))
        .find_map(|(key, value)| (key == "target" || key == "session").then_some(value))
}

fn servecore_ws_connection_guard(max_connections: usize) -> Option<ServecoreWsConnectionGuard> {
    let mut current = SERVECORE_WS_CONNECTIONS.load(Ordering::Relaxed);
    loop {
        if current >= max_connections {
            return None;
        }
        match SERVECORE_WS_CONNECTIONS.compare_exchange_weak(
            current,
            current + 1,
            Ordering::AcqRel,
            Ordering::Relaxed,
        ) {
            Ok(_) => return Some(ServecoreWsConnectionGuard),
            Err(actual) => current = actual,
        }
    }
}

struct ServecoreWsConnectionGuard;

impl Drop for ServecoreWsConnectionGuard {
    fn drop(&mut self) {
        SERVECORE_WS_CONNECTIONS.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use std::{net::Ipv4Addr, time::Duration};
    use tokio::sync::oneshot;

    async fn servecore_spawn_test_server() -> std::net::SocketAddr {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let app = servecore_apply_pipeline(servecore_mount_core_routes(Router::new()));
        let (tx, rx) = oneshot::channel::<()>();
        tokio::spawn(async move {
            let server = axum::serve(listener, app).with_graceful_shutdown(async move {
                let _ = rx.await;
            });
            server.await.expect("server");
        });
        std::mem::forget(tx);
        addr
    }

    async fn servecore_spawn_ws_test_server(
        state: ServecoreSharedState,
        config: modules::ws::WsConfig,
    ) -> std::net::SocketAddr {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let router = servecore_mount_core_routes(Router::new());
        let router = servecore_mount_ws_routes_with_config(router, config);
        let router = servecore_with_shared_state(router, state);
        let app = servecore_apply_pipeline_with_views_config(
            router,
            modules::views::ViewsConfig::views_with_paths(
                std::env::temp_dir().join("maw-rs-ws-no-ui"),
                std::env::temp_dir().join("maw-rs-ws-no-door.html"),
                std::env::temp_dir().join("maw-rs-ws-no-topology.html"),
            ),
        );
        let (tx, rx) = oneshot::channel::<()>();
        tokio::spawn(async move {
            let server = axum::serve(listener, app).with_graceful_shutdown(async move {
                let _ = rx.await;
            });
            server.await.expect("server");
        });
        std::mem::forget(tx);
        addr
    }

    #[derive(Debug, Default)]
    struct TestEngine {
        opened: Mutex<Vec<(ServecoreWsKind, Option<String>)>>,
    }

    impl ServecoreEngine for TestEngine {
        fn servecore_engine_name(&self) -> &'static str {
            "test"
        }

        fn servecore_ws_open(
            &self,
            kind: ServecoreWsKind,
            target: Option<&str>,
        ) -> Result<(), String> {
            let mut guard = self
                .opened
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard.push((kind, target.map(ToOwned::to_owned)));
            Ok(())
        }

        fn servecore_ws_text(
            &self,
            kind: ServecoreWsKind,
            text: &str,
            target: Option<&str>,
        ) -> Option<String> {
            Some(format!("{kind:?}:{}:{text}", target.unwrap_or("none")))
        }
    }

    #[test]
    fn servecore_route_registry_rejects_duplicates_and_accepts_params() {
        let mut registry = ServecoreRouteRegistry::default();
        registry
            .servecore_register(Method::GET, "/api/agent/:id")
            .expect("first");
        let duplicate = registry.servecore_register(Method::GET, "/api/agent/:id");
        assert!(duplicate
            .expect_err("duplicate")
            .contains("duplicate route"));
        registry
            .servecore_register(Method::POST, "/api/agent/:id")
            .expect("method distinct");
        assert_eq!(registry.servecore_routes().len(), 2);
    }

    #[test]
    fn servecore_ws_registry_rejects_duplicates() {
        let mut registry = ServecoreWsRegistry::default();
        registry.servecore_register_ws("/ws").expect("ws");
        registry
            .servecore_register_ws_kind("/ws/pty", ServecoreWsKind::Pty)
            .expect("pty");
        registry
            .servecore_register_ws_kind("/ws/tmux", ServecoreWsKind::Tmux)
            .expect("tmux");
        assert!(registry
            .servecore_register_ws("/ws")
            .expect_err("dup")
            .contains("duplicate ws"));
        assert_eq!(
            registry.servecore_paths(),
            vec!["/ws", "/ws/pty", "/ws/tmux"]
        );
        assert_eq!(
            registry.servecore_handlers(),
            vec![
                ("/ws".to_owned(), ServecoreWsKind::Engine),
                ("/ws/pty".to_owned(), ServecoreWsKind::Pty),
                ("/ws/tmux".to_owned(), ServecoreWsKind::Tmux),
            ]
        );
    }

    #[test]
    fn servecore_lifecycle_sorts_by_weight_then_name_and_whitelists() {
        let modules = vec![
            ServecoreLifecycleModule {
                name: "triggers".to_owned(),
                weight: 20,
            },
            ServecoreLifecycleModule {
                name: "agents".to_owned(),
                weight: 10,
            },
            ServecoreLifecycleModule {
                name: "debug".to_owned(),
                weight: 10,
            },
        ];
        let enabled = ServecoreLifecycle::servecore_from_profile(
            modules,
            &["debug".to_owned(), "triggers".to_owned()],
        );
        assert_eq!(
            enabled.servecore_enabled_modules(),
            vec!["debug", "triggers"]
        );
    }

    #[test]
    fn servecore_pipeline_order_matches_maw_js_surface() {
        assert_eq!(
            servecore_pipeline_order(),
            [
                "cors-preflight",
                "ws-upgrade",
                "engine-proxy",
                "api-protected-auth",
                "registry",
                "api-public",
                "registry",
                "fallback-views",
            ]
        );
    }

    #[tokio::test]
    async fn servecore_default_denies_protected_paths_and_allows_public() {
        let addr = servecore_spawn_test_server().await;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("client");
        let protected = client
            .post(format!("http://{addr}/api/triggers/fire"))
            .send()
            .await
            .expect("protected");
        assert_eq!(protected.status(), StatusCode::FORBIDDEN);
        let plugins = client
            .post(format!("http://{addr}/api/plugins/reload"))
            .send()
            .await
            .expect("plugins");
        assert_eq!(plugins.status(), StatusCode::FORBIDDEN);
        let cleanup = client
            .post(format!("http://{addr}/api/worktrees/cleanup"))
            .send()
            .await
            .expect("cleanup");
        assert_eq!(cleanup.status(), StatusCode::FORBIDDEN);
        let public = client
            .get(format!("http://{addr}/api/serve-core/pipeline"))
            .send()
            .await
            .expect("public");
        assert_eq!(public.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn servecore_ws_uses_engine_hook_and_keeps_default_deny() {
        let engine = Arc::new(TestEngine::default());
        let state = ServecoreSharedState::default().servecore_with_engine(engine.clone());
        let addr = servecore_spawn_ws_test_server(state, modules::ws::WsConfig::default()).await;
        let url = format!("ws://{addr}/ws/tmux?target=nova:1.0");
        let (mut ws, _response) = tokio_tungstenite::connect_async(&url)
            .await
            .expect("connect websocket");
        ws.send(tokio_tungstenite::tungstenite::Message::Text(
            "hello".to_owned(),
        ))
        .await
        .expect("send");
        loop {
            let received = ws.next().await.expect("frame").expect("frame ok");
            if let tokio_tungstenite::tungstenite::Message::Text(text) = received {
                assert_eq!(text, "Tmux:nova:1.0:hello");
                break;
            }
        }
        assert_eq!(
            engine
                .opened
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_slice(),
            &[(ServecoreWsKind::Tmux, Some("nova:1.0".to_owned()))]
        );
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("client");
        let protected = client
            .post(format!("http://{addr}/api/triggers/fire"))
            .send()
            .await
            .expect("protected");
        assert_eq!(protected.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn servecore_ws_rejects_bad_tunnel_target_before_upgrade() {
        let addr = servecore_spawn_ws_test_server(
            ServecoreSharedState::default(),
            modules::ws::WsConfig::default(),
        )
        .await;
        let err = tokio_tungstenite::connect_async(format!("ws://{addr}/ws/tmux?target=-danger"))
            .await
            .expect_err("bad target must be rejected before upgrade");
        assert!(err.to_string().contains("400"));
    }

    #[tokio::test]
    async fn servecore_ws_idle_timeout_closes_dead_connection() {
        let config = modules::ws::WsConfig {
            idle_timeout: Duration::from_millis(80),
            heartbeat_interval: Duration::from_millis(20),
            send_timeout: Duration::from_millis(50),
            max_frame_bytes: 1024,
            max_connections: 8,
        };
        let addr = servecore_spawn_ws_test_server(ServecoreSharedState::default(), config).await;
        let (mut ws, _response) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
            .await
            .expect("connect websocket");
        let close = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_))) = ws.next().await
                {
                    break;
                }
            }
        })
        .await;
        assert!(close.is_ok());
    }
}
