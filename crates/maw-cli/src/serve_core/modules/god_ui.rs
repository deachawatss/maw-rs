use super::ServecoreModuleRegistration;
use crate::serve_core::{ServecoreAgentPane, ServecoreLifecycleModule, ServecoreSharedState};
use axum::{response::IntoResponse, routing::get, Extension, Json, Router};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

const GODUI_TEAM_RECENT_MS: u64 = 2 * 60 * 60 * 1_000;

#[must_use]
pub fn godui_lifecycle_module() -> ServecoreLifecycleModule {
    ServecoreLifecycleModule {
        name: "god-ui".to_owned(),
        weight: 50,
    }
}

#[must_use]
pub fn godui_registration<S>() -> ServecoreModuleRegistration<S>
where
    S: Clone + Send + Sync + 'static,
{
    ServecoreModuleRegistration {
        lifecycle: godui_lifecycle_module(),
        mount: godui_mount,
    }
}

pub fn godui_mount<S>(router: Router<S>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    router
        .route("/api/costs", get(godui_costs_get))
        .route("/api/teams", get(godui_teams_get))
        .route("/api/ui-state", get(godui_ui_state_get))
        .route("/api/asks", get(godui_asks_get))
        .route("/api/pin-info", get(godui_pin_info_get))
}

async fn godui_costs_get() -> impl IntoResponse {
    Json(godui_costs_payload()).into_response()
}

async fn godui_teams_get(
    Extension(state): Extension<Arc<ServecoreSharedState>>,
) -> impl IntoResponse {
    Json(godui_teams_payload(&state)).into_response()
}

async fn godui_ui_state_get() -> impl IntoResponse {
    Json(godui_ui_state_payload()).into_response()
}

async fn godui_asks_get() -> impl IntoResponse {
    Json(godui_asks_payload()).into_response()
}

async fn godui_pin_info_get() -> impl IntoResponse {
    Json(godui_pin_info_payload()).into_response()
}

fn godui_costs_payload() -> Value {
    json!({
        "agents": [],
        "total": {
            "tokens": 0,
            "cost": 0.0,
            "sessions": 0,
            "agents": 0
        }
    })
}

fn godui_teams_payload(state: &ServecoreSharedState) -> GoduiTeamsResponse {
    let home = godui_home_dir();
    let teams = godui_scan_teams(
        &home.join(".claude").join("teams"),
        &home.join(".claude").join("tasks"),
        &home,
        &state.servecore_agents_panes(),
        godui_now_millis(),
    );
    GoduiTeamsResponse {
        total: teams.len(),
        teams,
    }
}

fn godui_ui_state_payload() -> Value {
    godui_read_json(&godui_current_dir_file("ui-state.json")).unwrap_or_else(|| json!({}))
}

fn godui_asks_payload() -> Value {
    godui_read_json(&godui_current_dir_file("asks.json")).unwrap_or_else(|| json!([]))
}

fn godui_pin_info_payload() -> GoduiPinInfoResponse {
    let config = maw_xdg::load_merged_config(&godui_xdg_env()).config;
    let pin = config
        .get("pin")
        .and_then(Value::as_str)
        .unwrap_or_default();
    GoduiPinInfoResponse {
        length: pin.chars().count(),
        enabled: !pin.is_empty(),
    }
}

#[derive(Clone, Debug, Serialize, PartialEq)]
struct GoduiTeamsResponse {
    teams: Vec<Value>,
    total: usize,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
struct GoduiPinInfoResponse {
    length: usize,
    enabled: bool,
}

fn godui_scan_teams(
    teams_dir: &Path,
    tasks_root: &Path,
    home: &Path,
    panes: &[ServecoreAgentPane],
    now_ms: u64,
) -> Vec<Value> {
    let Ok(entries) = fs::read_dir(teams_dir) else {
        return Vec::new();
    };
    let live_pane_ids = panes
        .iter()
        .map(|pane| pane.id.clone())
        .collect::<BTreeSet<_>>();
    let mut teams = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            godui_read_team(&entry.path(), tasks_root, home, &live_pane_ids, now_ms)
        })
        .collect::<Vec<_>>();
    teams.sort_by(|left, right| left.0.cmp(&right.0));
    teams.into_iter().map(|(_, team)| team).collect()
}

fn godui_read_team(
    team_dir: &Path,
    tasks_root: &Path,
    home: &Path,
    live_pane_ids: &BTreeSet<String>,
    now_ms: u64,
) -> Option<(String, Value)> {
    let name = team_dir.file_name()?.to_string_lossy().into_owned();
    let config = godui_read_json(&team_dir.join("config.json"))?;
    let mut object = config.as_object().cloned()?;
    object
        .entry("name".to_owned())
        .or_insert_with(|| Value::String(name.clone()));
    let tasks = godui_read_tasks(&tasks_root.join(&name));
    let alive = godui_team_alive(object.get("members"), live_pane_ids, home, now_ms);
    object.insert("tasks".to_owned(), Value::Array(tasks));
    object.insert("alive".to_owned(), Value::Bool(alive));
    let sort_name = object
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(&name)
        .to_owned();
    Some((sort_name, Value::Object(object)))
}

fn godui_read_tasks(tasks_dir: &Path) -> Vec<Value> {
    let Ok(entries) = fs::read_dir(tasks_dir) else {
        return Vec::new();
    };
    let mut files = entries.filter_map(Result::ok).collect::<Vec<_>>();
    files.sort_by_key(std::fs::DirEntry::file_name);
    files
        .into_iter()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|entry| godui_read_json(&entry.path()))
        .collect()
}

fn godui_team_alive(
    members: Option<&Value>,
    live_pane_ids: &BTreeSet<String>,
    home: &Path,
    now_ms: u64,
) -> bool {
    members.and_then(Value::as_array).is_some_and(|members| {
        members
            .iter()
            .any(|member| godui_member_alive(member, live_pane_ids, home, now_ms))
    })
}

fn godui_member_alive(
    member: &Value,
    live_pane_ids: &BTreeSet<String>,
    home: &Path,
    now_ms: u64,
) -> bool {
    let Some(member) = member.as_object() else {
        return false;
    };
    let backend = godui_field_str(member, "backendType");
    let cwd = godui_field_str(member, "cwd");
    let joined_at = member.get("joinedAt").and_then(Value::as_u64);
    if backend == Some("tmux")
        && godui_field_str(member, "tmuxPaneId").is_some_and(|pane| live_pane_ids.contains(pane))
    {
        return true;
    }
    if backend == Some("in-process") && godui_recent_local(cwd, joined_at, home, now_ms) {
        return true;
    }
    let agent_type = godui_field_str(member, "agentType");
    let name = godui_field_str(member, "name");
    (agent_type == Some("team-lead") || name == Some("team-lead"))
        && godui_recent_local(cwd, joined_at, home, now_ms)
}

fn godui_recent_local(cwd: Option<&str>, joined_at: Option<u64>, home: &Path, now_ms: u64) -> bool {
    let Some(cwd) = cwd else {
        return false;
    };
    if !Path::new(cwd).starts_with(home) {
        return false;
    }
    joined_at.is_some_and(|joined_at| now_ms.saturating_sub(joined_at) < GODUI_TEAM_RECENT_MS)
}

fn godui_field_str<'a>(object: &'a Map<String, Value>, field: &str) -> Option<&'a str> {
    object.get(field).and_then(Value::as_str)
}

fn godui_read_json(path: &Path) -> Option<Value> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn godui_current_dir_file(name: &str) -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(name)
}

fn godui_home_dir() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("."), PathBuf::from)
}

fn godui_xdg_env() -> maw_xdg::MawXdgEnv {
    let vars = [
        "MAW_HOME",
        "MAW_CONFIG_DIR",
        "MAW_XDG",
        "XDG_CONFIG_HOME",
        "XDG_STATE_HOME",
        "MAW_STATE_DIR",
        "XDG_DATA_HOME",
        "MAW_DATA_DIR",
        "XDG_CACHE_HOME",
        "MAW_CACHE_DIR",
    ]
    .into_iter()
    .filter_map(|key| std::env::var(key).ok().map(|value| (key.to_owned(), value)));
    maw_xdg::MawXdgEnv::with_vars(godui_home_dir(), vars)
}

fn godui_now_millis() -> u64 {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serve_core::{
        modules::servecore_mount_modules, servecore_apply_pipeline, servecore_mount_core_routes,
        servecore_with_shared_state,
    };
    use axum::{
        body::Body,
        http::{Method, Request, StatusCode},
    };
    use std::time::Duration;
    use tower::ServiceExt;

    #[test]
    fn godui_empty_payloads_match_maw_js_shapes() {
        let costs = godui_costs_payload();
        assert_eq!(costs["agents"], json!([]));
        assert_eq!(costs["total"]["tokens"], 0);
        assert_eq!(costs["total"]["cost"], 0.0);
        assert_eq!(costs["total"]["sessions"], 0);
        assert_eq!(costs["total"]["agents"], 0);
    }

    #[test]
    fn godui_scan_teams_merges_config_tasks_and_liveness() {
        let root = godui_test_root("teams");
        let home = root.join("home");
        let teams_dir = home.join(".claude/teams");
        let tasks_root = home.join(".claude/tasks");
        fs::create_dir_all(teams_dir.join("alpha")).expect("team dir");
        fs::create_dir_all(tasks_root.join("alpha")).expect("tasks dir");
        fs::write(
            teams_dir.join("alpha/config.json"),
            json!({
                "name": "alpha",
                "description": "demo",
                "members": [
                    {"name": "builder", "backendType": "tmux", "tmuxPaneId": "%9"}
                ]
            })
            .to_string(),
        )
        .expect("config");
        fs::write(
            tasks_root.join("alpha/1.json"),
            json!({"id": 1, "status": "in_progress"}).to_string(),
        )
        .expect("task");
        let panes = vec![ServecoreAgentPane {
            id: "%9".to_owned(),
            command: "codex".to_owned(),
            target: "alpha:1.0".to_owned(),
            title: "builder-agent".to_owned(),
            cwd: Some(home.to_string_lossy().into_owned()),
            pid: Some(99),
            last_activity: Some(1),
        }];

        let teams = godui_scan_teams(&teams_dir, &tasks_root, &home, &panes, 1_000);

        assert_eq!(teams.len(), 1);
        assert_eq!(teams[0]["name"], "alpha");
        assert_eq!(teams[0]["tasks"][0]["id"], 1);
        assert_eq!(teams[0]["alive"], true);
        fs::remove_dir_all(root).ok();
    }

    #[tokio::test]
    async fn godui_routes_return_200_json_and_cors_headers() {
        let state = ServecoreSharedState::default().servecore_with_agents_snapshot(Vec::new());
        let router = servecore_mount_core_routes(Router::new());
        let router = servecore_mount_modules(router, &["god-ui".to_owned()]);
        let router = servecore_with_shared_state(router, state);
        let app = servecore_apply_pipeline(router);
        let endpoints = [
            "/api/costs",
            "/api/teams",
            "/api/ui-state",
            "/api/asks",
            "/api/pin-info",
        ];

        for endpoint in endpoints {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(Method::GET)
                        .uri(endpoint)
                        .header("origin", "https://god.buildwithoracle.com")
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(response.status(), StatusCode::OK, "{endpoint}");
            assert_eq!(
                response.headers().get("access-control-allow-origin"),
                Some(&"https://god.buildwithoracle.com".parse().expect("origin")),
                "{endpoint}"
            );
            assert!(response
                .headers()
                .get("content-type")
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.starts_with("application/json")));
        }
    }

    fn godui_test_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_else(|_| Duration::from_secs(0))
            .as_nanos();
        std::env::temp_dir().join(format!(
            "maw-rs-god-ui-{name}-{}-{unique}",
            std::process::id()
        ))
    }
}
