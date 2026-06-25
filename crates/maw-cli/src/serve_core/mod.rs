pub mod modules;

use axum::{
    body::Body,
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    http::{Method, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{any, get, post},
    Extension, Json, Router,
};
use maw_hub::WorkspaceConfig;
use maw_tmux::{TmuxClient, TmuxPane};
use serde_json::json;
use std::{
    collections::{BTreeSet, VecDeque},
    sync::{Arc, Mutex},
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

pub trait ServecoreEngine: Send + Sync {
    fn servecore_engine_name(&self) -> &'static str;
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
    handlers: BTreeSet<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServecoreWsKind {
    EchoSkeleton,
}

impl ServecoreWsRegistry {
    /// Register one websocket upgrade path.
    ///
    /// # Errors
    /// Returns an error when the path is invalid or already registered.
    pub fn servecore_register_ws(&mut self, path: &str) -> Result<(), String> {
        servecore_validate_path(path)?;
        if !self.handlers.insert(path.to_owned()) {
            return Err(format!("serve-core: duplicate ws route {path}"));
        }
        Ok(())
    }

    #[must_use]
    pub fn servecore_paths(&self) -> Vec<String> {
        self.handlers.iter().cloned().collect()
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
    router
        .route("/ws", get(servecore_ws_upgrade))
        .route("/ws/pty", get(servecore_ws_upgrade))
        .route("/ws/tmux", get(servecore_ws_upgrade))
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

async fn servecore_ws_upgrade(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(servecore_ws_echo)
}

async fn servecore_ws_echo(mut socket: WebSocket) {
    while let Some(frame) = socket.recv().await {
        match frame {
            Ok(Message::Text(text)) => {
                if socket.send(Message::Text(text)).await.is_err() {
                    break;
                }
            }
            Ok(Message::Binary(bytes)) => {
                if socket.send(Message::Binary(bytes)).await.is_err() {
                    break;
                }
            }
            Ok(Message::Ping(bytes)) => {
                if socket.send(Message::Pong(bytes)).await.is_err() {
                    break;
                }
            }
            Ok(Message::Close(frame)) => {
                let _ = socket.send(Message::Close(frame)).await;
                break;
            }
            Ok(Message::Pong(_)) => {}
            Err(_) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        registry.servecore_register_ws("/ws/pty").expect("pty");
        registry.servecore_register_ws("/ws/tmux").expect("tmux");
        assert!(registry
            .servecore_register_ws("/ws")
            .expect_err("dup")
            .contains("duplicate ws"));
        assert_eq!(
            registry.servecore_paths(),
            vec!["/ws", "/ws/pty", "/ws/tmux"]
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
}
