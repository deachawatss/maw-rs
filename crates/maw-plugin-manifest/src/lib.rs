//! Plugin manifest validators ported from maw-js `src/plugin/manifest-validate.ts`.
//!
//! This crate locks the same optional-field validator contracts covered by
//! `test/plugin-manifest-validate-edges.test.ts`.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginCli {
    pub command: String,
    pub aliases: Option<Vec<String>>,
    pub help: Option<String>,
    pub flags: Option<BTreeMap<String, CliFlagKind>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliFlagKind {
    Boolean,
    String,
    Number,
}

impl CliFlagKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Boolean => "boolean",
            Self::String => "string",
            Self::Number => "number",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginApi {
    pub path: String,
    pub methods: Vec<ApiMethod>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiMethod {
    Get,
    Post,
}

impl ApiMethod {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginHooks {
    pub gate: Option<Vec<String>>,
    pub filter: Option<Vec<String>>,
    pub on: Option<Vec<String>>,
    pub late: Option<Vec<String>>,
    pub wake: Option<PluginLifecycleHook>,
    pub sleep: Option<PluginLifecycleHook>,
    pub serve: Option<PluginLifecycleHook>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginLifecycleHook {
    pub script: Option<String>,
    pub handler: Option<String>,
    pub ensures: Option<Vec<String>>,
    pub policy: Option<HookPolicy>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookPolicy {
    BestEffort,
    FailFast,
}

impl HookPolicy {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BestEffort => "best-effort",
            Self::FailFast => "fail-fast",
        }
    }
}

/// Parse the optional `cli` section.
///
/// # Errors
///
/// Returns maw-js-compatible validation messages for malformed `cli` shapes.
pub fn parse_cli(manifest: &Value) -> Result<Option<PluginCli>, String> {
    let Some(cli) = manifest.get("cli") else {
        return Ok(None);
    };
    let Some(cli) = cli.as_object() else {
        return Err("plugin.json: cli must be an object".to_owned());
    };

    let Some(command) = cli
        .get("command")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    else {
        return Err("plugin.json: cli.command must be a non-empty string".to_owned());
    };

    let aliases = if let Some(aliases) = cli.get("aliases") {
        Some(parse_string_array(
            aliases,
            "plugin.json: cli.aliases must be an array of strings",
            false,
        )?)
    } else {
        None
    };

    let flags = if let Some(flags) = cli.get("flags") {
        let Some(values) = flags.as_object() else {
            return Err("plugin.json: cli.flags must be an object".to_owned());
        };
        let mut parsed = BTreeMap::new();
        for (key, value) in values {
            let Some(raw) = value.as_str() else {
                return Err(format!(
                    "plugin.json: cli.flags[\"{key}\"] must be \"boolean\", \"string\", or \"number\""
                ));
            };
            let kind = match raw {
                "boolean" => CliFlagKind::Boolean,
                "string" => CliFlagKind::String,
                "number" => CliFlagKind::Number,
                _ => {
                    return Err(format!(
                        "plugin.json: cli.flags[\"{key}\"] must be \"boolean\", \"string\", or \"number\""
                    ));
                }
            };
            parsed.insert(key.clone(), kind);
        }
        Some(parsed)
    } else {
        None
    };

    Ok(Some(PluginCli {
        command: command.to_owned(),
        aliases,
        help: cli.get("help").and_then(Value::as_str).map(str::to_owned),
        flags,
    }))
}

/// Parse the optional `api` section.
///
/// # Errors
///
/// Returns maw-js-compatible validation messages for malformed `api` shapes.
pub fn parse_api(manifest: &Value) -> Result<Option<PluginApi>, String> {
    let Some(api) = manifest.get("api") else {
        return Ok(None);
    };
    let Some(api) = api.as_object() else {
        return Err("plugin.json: api must be an object".to_owned());
    };

    let Some(path) = api
        .get("path")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    else {
        return Err("plugin.json: api.path must be a non-empty string".to_owned());
    };

    let Some(methods) = api.get("methods").and_then(Value::as_array) else {
        return Err("plugin.json: api.methods must be an array of \"GET\" | \"POST\"".to_owned());
    };
    let mut parsed = Vec::with_capacity(methods.len());
    for method in methods {
        let parsed_method = match method.as_str() {
            Some("GET") => ApiMethod::Get,
            Some("POST") => ApiMethod::Post,
            _ => {
                return Err(
                    "plugin.json: api.methods must be an array of \"GET\" | \"POST\"".to_owned(),
                );
            }
        };
        parsed.push(parsed_method);
    }

    Ok(Some(PluginApi {
        path: path.to_owned(),
        methods: parsed,
    }))
}

/// Parse the optional `hooks` section.
///
/// # Errors
///
/// Returns maw-js-compatible validation messages for malformed hook shapes.
pub fn parse_hooks(manifest: &Value) -> Result<Option<PluginHooks>, String> {
    let Some(hooks) = manifest.get("hooks") else {
        return Ok(None);
    };
    let Some(hooks) = hooks.as_object() else {
        return Err("plugin.json: hooks must be an object".to_owned());
    };

    let gate = parse_optional_string_array(
        hooks,
        "gate",
        "plugin.json: hooks.gate must be an array of strings",
        false,
    )?;
    let filter = parse_optional_string_array(
        hooks,
        "filter",
        "plugin.json: hooks.filter must be an array of strings",
        false,
    )?;
    let on = parse_optional_string_array(
        hooks,
        "on",
        "plugin.json: hooks.on must be an array of strings",
        false,
    )?;
    let late = parse_optional_string_array(
        hooks,
        "late",
        "plugin.json: hooks.late must be an array of strings",
        false,
    )?;

    Ok(Some(PluginHooks {
        gate,
        filter,
        on,
        late,
        wake: parse_lifecycle_hook(hooks, "wake")?,
        sleep: parse_lifecycle_hook(hooks, "sleep")?,
        serve: parse_lifecycle_hook(hooks, "serve")?,
    }))
}

fn parse_lifecycle_hook(
    hooks: &Map<String, Value>,
    key: &'static str,
) -> Result<Option<PluginLifecycleHook>, String> {
    let Some(raw) = hooks.get(key) else {
        return Ok(None);
    };
    let Some(hook) = raw.as_object() else {
        return Err(format!("plugin.json: hooks.{key} must be an object"));
    };

    let script = match hook.get("script") {
        Some(value) => Some(
            value
                .as_str()
                .filter(|script| !script.is_empty())
                .ok_or_else(|| {
                    format!("plugin.json: hooks.{key}.script must be a non-empty string")
                })?
                .to_owned(),
        ),
        None => None,
    };

    let handler = match hook.get("handler") {
        Some(value) => Some(
            value
                .as_str()
                .filter(|handler| !handler.is_empty())
                .ok_or_else(|| {
                    format!("plugin.json: hooks.{key}.handler must be a non-empty string")
                })?
                .to_owned(),
        ),
        None => None,
    };

    let ensures = parse_optional_string_array(
        hook,
        "ensures",
        &format!("plugin.json: hooks.{key}.ensures must be an array of non-empty strings"),
        true,
    )?;

    let policy = match hook.get("policy") {
        Some(Value::String(value)) if value == "best-effort" => Some(HookPolicy::BestEffort),
        Some(Value::String(value)) if value == "fail-fast" => Some(HookPolicy::FailFast),
        Some(_) => {
            return Err(format!(
                "plugin.json: hooks.{key}.policy must be \"best-effort\" or \"fail-fast\""
            ));
        }
        None => None,
    };

    Ok(Some(PluginLifecycleHook {
        script,
        handler,
        ensures,
        policy,
    }))
}

fn parse_optional_string_array(
    object: &Map<String, Value>,
    key: &str,
    error: &str,
    reject_empty: bool,
) -> Result<Option<Vec<String>>, String> {
    object
        .get(key)
        .map(|value| parse_string_array(value, error, reject_empty))
        .transpose()
}

fn parse_string_array(
    value: &Value,
    error: &str,
    reject_empty: bool,
) -> Result<Vec<String>, String> {
    let Some(values) = value.as_array() else {
        return Err(error.to_owned());
    };
    let mut parsed = Vec::with_capacity(values.len());
    for value in values {
        let Some(item) = value.as_str() else {
            return Err(error.to_owned());
        };
        if reject_empty && item.is_empty() {
            return Err(error.to_owned());
        }
        parsed.push(item.to_owned());
    }
    Ok(parsed)
}
