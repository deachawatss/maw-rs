//! Hub workspace configuration helpers ported from maw-js `src/transports/hub-config.ts`.
//!
//! The crate keeps config loading deterministic for tests by taking an explicit config
//! directory instead of reading process globals directly.

use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

pub const HEARTBEAT_MS: u64 = 30_000;
pub const RECONNECT_BASE_MS: u64 = 1_000;
pub const RECONNECT_MAX_MS: u64 = 60_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceConfig {
    pub id: String,
    pub hub_url: String,
    pub token: String,
    pub shared_agents: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceConfigValidation {
    Ok,
    Invalid { reason: String },
}

impl WorkspaceConfigValidation {
    #[must_use]
    pub const fn ok(&self) -> bool {
        matches!(self, Self::Ok)
    }

    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Ok => None,
            Self::Invalid { reason } => Some(reason.as_str()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceLoadReport {
    pub configs: Vec<WorkspaceConfig>,
    pub warnings: Vec<String>,
}

#[must_use]
pub fn workspaces_dir(config_dir: impl AsRef<Path>) -> std::path::PathBuf {
    config_dir.as_ref().join("workspaces")
}

#[must_use]
pub fn validate_workspace_config(raw: &serde_json::Value) -> WorkspaceConfigValidation {
    let Some(obj) = raw.as_object() else {
        return invalid("not an object");
    };
    if !non_empty_string(obj.get("id")) {
        return invalid("missing/empty id");
    }
    if !non_empty_string(obj.get("hubUrl")) {
        return invalid("missing/empty hubUrl");
    }
    if !non_empty_string(obj.get("token")) {
        return invalid("missing/empty token");
    }
    if !matches!(obj.get("sharedAgents"), Some(serde_json::Value::Array(_))) {
        return invalid("sharedAgents must be array");
    }
    let hub_url = obj
        .get("hubUrl")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    match websocket_protocol(hub_url) {
        Some("ws" | "wss") => WorkspaceConfigValidation::Ok,
        Some(protocol) => invalid(format!("hubUrl must be ws:|wss: (got {protocol}:)")),
        None => invalid("hubUrl not a valid URL"),
    }
}

/// Load valid workspace configs from `<config_dir>/workspaces/*.json`.
///
/// # Errors
///
/// Returns filesystem errors from directory creation or enumeration. Individual
/// malformed files are skipped and reported in `WorkspaceLoadReport::warnings`,
/// matching maw-js's warning-and-continue behavior.
pub fn load_workspace_configs(
    config_dir: impl AsRef<Path>,
) -> std::io::Result<WorkspaceLoadReport> {
    let dir = workspaces_dir(config_dir);
    if !dir.exists() {
        fs::create_dir_all(&dir)?;
        return Ok(WorkspaceLoadReport {
            configs: Vec::new(),
            warnings: Vec::new(),
        });
    }

    let mut files = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            files.push(path);
        }
    }
    files.sort();

    let mut configs = Vec::new();
    let mut warnings = Vec::new();
    for path in files {
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("<unknown>")
            .to_owned();
        let raw = match fs::read_to_string(&path).and_then(|text| {
            serde_json::from_str::<serde_json::Value>(&text)
                .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))
        }) {
            Ok(raw) => raw,
            Err(err) => {
                warnings.push(format!(
                    "[hub] failed to parse workspace config: {file_name} {err}"
                ));
                continue;
            }
        };
        match validate_workspace_config(&raw) {
            WorkspaceConfigValidation::Ok => match serde_json::from_value::<WorkspaceConfig>(raw) {
                Ok(config) => configs.push(config),
                Err(err) => warnings.push(format!(
                    "[hub] failed to parse workspace config: {file_name} {err}"
                )),
            },
            WorkspaceConfigValidation::Invalid { reason } => warnings.push(format!(
                "[hub] invalid workspace config: {file_name} ({reason})"
            )),
        }
    }

    Ok(WorkspaceLoadReport { configs, warnings })
}

fn invalid(reason: impl Into<String>) -> WorkspaceConfigValidation {
    WorkspaceConfigValidation::Invalid {
        reason: reason.into(),
    }
}

fn non_empty_string(value: Option<&serde_json::Value>) -> bool {
    value
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| !value.is_empty())
}

fn websocket_protocol(url: &str) -> Option<&str> {
    let (protocol, rest) = url.split_once("://")?;
    if protocol.is_empty() || rest.is_empty() || rest.contains(char::is_whitespace) {
        return None;
    }
    Some(protocol)
}
