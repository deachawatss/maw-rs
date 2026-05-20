//! Plugin manifest validators ported from maw-js `src/plugin/manifest-validate.ts`.
//!
//! This crate locks the same optional-field validator contracts covered by
//! `test/plugin-manifest-validate-edges.test.ts`.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

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
pub struct PluginCron {
    pub schedule: String,
    pub handler: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginModule {
    pub exports: Vec<String>,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginTransport {
    pub peer: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginEngine {
    pub serve: Option<PluginEngineServe>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginEngineServe {
    pub command: Option<String>,
    pub prefix: Option<String>,
    pub health: Option<String>,
    pub events: Option<Vec<String>>,
    pub event_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginTarget {
    Js,
}

impl PluginTarget {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Js => "js",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginTier {
    Core,
    Standard,
    Extra,
}

impl PluginTier {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::Standard => "standard",
            Self::Extra => "extra",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginDependencies {
    pub plugins: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginArtifact {
    pub path: String,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginCapabilities {
    pub capabilities: Vec<String>,
    pub warnings: Vec<String>,
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

/// Parse the optional `cron` section.
///
/// # Errors
///
/// Returns maw-js-compatible validation messages for malformed cron shapes.
pub fn parse_cron(manifest: &Value) -> Result<Option<PluginCron>, String> {
    let Some(cron) = manifest.get("cron") else {
        return Ok(None);
    };
    let Some(cron) = cron.as_object() else {
        return Err("plugin.json: cron must be an object".to_owned());
    };

    let Some(schedule) = cron
        .get("schedule")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    else {
        return Err("plugin.json: cron.schedule must be a non-empty string".to_owned());
    };

    let handler = match cron.get("handler") {
        Some(value) => Some(
            value
                .as_str()
                .ok_or_else(|| "plugin.json: cron.handler must be a string".to_owned())?
                .to_owned(),
        ),
        None => None,
    };

    Ok(Some(PluginCron {
        schedule: schedule.to_owned(),
        handler,
    }))
}

/// Parse the optional `module` section.
///
/// # Errors
///
/// Returns maw-js-compatible validation messages for malformed module shapes.
pub fn parse_module(manifest: &Value) -> Result<Option<PluginModule>, String> {
    let Some(module) = manifest.get("module") else {
        return Ok(None);
    };
    let Some(module) = module.as_object() else {
        return Err("plugin.json: module must be an object".to_owned());
    };

    let exports = parse_string_array(
        module.get("exports").ok_or_else(|| {
            "plugin.json: module.exports must be a non-empty array of strings".to_owned()
        })?,
        "plugin.json: module.exports must be a non-empty array of strings",
        false,
    )?;
    if exports.is_empty() {
        return Err("plugin.json: module.exports must be a non-empty array of strings".to_owned());
    }

    let Some(path) = module
        .get("path")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    else {
        return Err("plugin.json: module.path must be a non-empty string".to_owned());
    };

    Ok(Some(PluginModule {
        exports,
        path: path.to_owned(),
    }))
}

/// Parse the optional `transport` section.
///
/// # Errors
///
/// Returns maw-js-compatible validation messages for malformed transport shapes.
pub fn parse_transport(manifest: &Value) -> Result<Option<PluginTransport>, String> {
    let Some(transport) = manifest.get("transport") else {
        return Ok(None);
    };
    let Some(transport) = transport.as_object() else {
        return Err("plugin.json: transport must be an object".to_owned());
    };

    let peer = match transport.get("peer") {
        Some(value) => Some(
            value
                .as_bool()
                .ok_or_else(|| "plugin.json: transport.peer must be a boolean".to_owned())?,
        ),
        None => None,
    };

    Ok(Some(PluginTransport { peer }))
}

/// Parse the optional `engine` section.
///
/// # Errors
///
/// Returns maw-js-compatible validation messages for malformed engine serve metadata.
pub fn parse_engine(manifest: &Value) -> Result<Option<PluginEngine>, String> {
    let Some(engine) = manifest.get("engine") else {
        return Ok(None);
    };
    let Some(engine) = engine.as_object() else {
        return Err("plugin.json: engine must be an object".to_owned());
    };
    let Some(serve) = engine.get("serve") else {
        return Ok(Some(PluginEngine { serve: None }));
    };
    let Some(serve) = serve.as_object() else {
        return Err("plugin.json: engine.serve must be an object".to_owned());
    };

    let command = match serve.get("command") {
        Some(value) => Some(
            value
                .as_str()
                .filter(|command| !command.is_empty())
                .ok_or_else(|| {
                    "plugin.json: engine.serve.command must be a non-empty string".to_owned()
                })?
                .to_owned(),
        ),
        None => None,
    };

    let prefix = match serve.get("prefix") {
        Some(value) => Some(
            value
                .as_str()
                .filter(|prefix| prefix.starts_with("/api/"))
                .ok_or_else(|| "plugin.json: engine.serve.prefix must start with /api/".to_owned())?
                .to_owned(),
        ),
        None => None,
    };

    let health = match serve.get("health") {
        Some(value) => Some(
            value
                .as_str()
                .filter(|health| health.starts_with('/'))
                .ok_or_else(|| {
                    "plugin.json: engine.serve.health must be an absolute path".to_owned()
                })?
                .to_owned(),
        ),
        None => None,
    };

    let events = parse_optional_string_array(
        serve,
        "events",
        "plugin.json: engine.serve.events must be an array of non-empty strings",
        true,
    )?;

    let event_path = match serve.get("eventPath") {
        Some(value) => Some(
            value
                .as_str()
                .filter(|event_path| event_path.starts_with('/'))
                .ok_or_else(|| {
                    "plugin.json: engine.serve.eventPath must be an absolute path".to_owned()
                })?
                .to_owned(),
        ),
        None => None,
    };

    Ok(Some(PluginEngine {
        serve: Some(PluginEngineServe {
            command,
            prefix,
            health,
            events,
            event_path,
        }),
    }))
}

/// Parse the optional `target` section.
///
/// # Errors
///
/// Returns maw-js-compatible validation messages for unsupported targets.
pub fn parse_target(manifest: &Value) -> Result<Option<PluginTarget>, String> {
    let Some(target) = manifest.get("target") else {
        return Ok(None);
    };
    let Some(target_string) = target.as_str() else {
        return Err("plugin.json: target must be a string".to_owned());
    };
    if target_string == "wasm" {
        return Err(
            "plugin.json: target \"wasm\" not yet supported (Phase C). Use target \"js\" for now."
                .to_owned(),
        );
    }
    if target_string != "js" {
        return Err(format!(
            "plugin.json: unknown target {target} (expected \"js\")"
        ));
    }
    Ok(Some(PluginTarget::Js))
}

/// Parse optional `capabilityNamespaces`.
///
/// # Errors
///
/// Returns maw-js-compatible validation messages for malformed namespace arrays.
pub fn parse_capability_namespaces(manifest: &Value) -> Result<Option<Vec<String>>, String> {
    let Some(namespaces) = manifest.get("capabilityNamespaces") else {
        return Ok(None);
    };
    let namespaces = parse_string_array(
        namespaces,
        "plugin.json: capabilityNamespaces must be an array of slug strings",
        true,
    )?;
    if namespaces.iter().any(|namespace| !is_slug(namespace)) {
        return Err(
            "plugin.json: capabilityNamespaces must be an array of slug strings".to_owned(),
        );
    }

    let mut deduped = Vec::new();
    for namespace in namespaces {
        if !deduped.contains(&namespace) {
            deduped.push(namespace);
        }
    }
    Ok(Some(deduped))
}

/// Parse optional `capabilities` and collect maw-js warning text for unknown namespaces.
///
/// # Errors
///
/// Returns maw-js-compatible validation messages for malformed capability arrays.
pub fn parse_capabilities(
    manifest: &Value,
    extra_namespaces: &[&str],
) -> Result<Option<PluginCapabilities>, String> {
    let Some(capabilities) = manifest.get("capabilities") else {
        return Ok(None);
    };
    let capabilities = parse_string_array(
        capabilities,
        "plugin.json: capabilities must be an array of strings",
        false,
    )?;
    let mut warnings = Vec::new();
    for capability in &capabilities {
        let namespace = capability
            .split_once(':')
            .map_or(capability.as_str(), |(namespace, _)| namespace);
        if !is_known_capability_namespace(namespace)
            && !extra_namespaces.iter().any(|extra| extra == &namespace)
        {
            let mut known = known_capability_namespaces();
            known.extend(extra_namespaces.iter().copied());
            warnings.push(format!(
                "plugin.json: unknown capability namespace \"{namespace}\" in \"{capability}\" (known: {})",
                known.join(", ")
            ));
        }
    }
    Ok(Some(PluginCapabilities {
        capabilities,
        warnings,
    }))
}

/// Parse optional `dependencies`.
///
/// # Errors
///
/// Returns maw-js-compatible validation messages for malformed dependency shapes.
pub fn parse_dependencies(manifest: &Value) -> Result<Option<PluginDependencies>, String> {
    let Some(dependencies) = manifest.get("dependencies") else {
        return Ok(None);
    };

    let plugins_value = if dependencies.is_array() {
        Some(dependencies)
    } else if let Some(object) = dependencies.as_object() {
        object.get("plugins")
    } else {
        return Err(
            "plugin.json: dependencies must be an object or array of plugin names".to_owned(),
        );
    };

    let Some(plugins_value) = plugins_value else {
        return Ok(Some(PluginDependencies { plugins: None }));
    };
    let plugins = parse_string_array(
        plugins_value,
        "plugin.json: dependencies.plugins must be an array of plugin names",
        true,
    )?;
    if plugins.iter().any(|plugin| !is_slug(plugin)) {
        return Err(
            "plugin.json: dependencies.plugins must be an array of plugin names".to_owned(),
        );
    }
    Ok(Some(PluginDependencies {
        plugins: Some(plugins),
    }))
}

/// Parse optional `artifact`.
///
/// # Errors
///
/// Returns maw-js-compatible validation messages for malformed artifact shapes.
pub fn parse_artifact(manifest: &Value) -> Result<Option<PluginArtifact>, String> {
    let Some(artifact) = manifest.get("artifact") else {
        return Ok(None);
    };
    let Some(artifact) = artifact.as_object() else {
        return Err("plugin.json: artifact must be an object".to_owned());
    };

    let Some(path) = artifact
        .get("path")
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty())
    else {
        return Err("plugin.json: artifact.path must be a non-empty string".to_owned());
    };

    let sha256_value = artifact
        .get("sha256")
        .ok_or_else(|| "plugin.json: artifact.sha256 must be a string or null".to_owned())?;
    let sha256 = if sha256_value.is_null() {
        None
    } else {
        Some(
            sha256_value
                .as_str()
                .ok_or_else(|| "plugin.json: artifact.sha256 must be a string or null".to_owned())?
                .to_owned(),
        )
    };

    Ok(Some(PluginArtifact {
        path: path.to_owned(),
        sha256,
    }))
}

/// Parse optional `tier`.
///
/// # Errors
///
/// Returns maw-js-compatible validation messages for unknown tiers.
pub fn parse_tier(manifest: &Value) -> Result<Option<PluginTier>, String> {
    let Some(tier) = manifest.get("tier") else {
        return Ok(None);
    };
    let Some(tier_string) = tier.as_str() else {
        return Err(format!(
            "plugin.json: tier must be \"core\", \"standard\", or \"extra\" (got {tier})"
        ));
    };
    let tier = match tier_string {
        "core" => PluginTier::Core,
        "standard" => PluginTier::Standard,
        "extra" => PluginTier::Extra,
        _ => {
            return Err(format!(
                "plugin.json: tier must be \"core\", \"standard\", or \"extra\" (got {tier})"
            ));
        }
    };
    Ok(Some(tier))
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

fn is_slug(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn is_known_capability_namespace(namespace: &str) -> bool {
    known_capability_namespaces().contains(&namespace)
}

fn known_capability_namespaces() -> Vec<&'static str> {
    vec![
        "net", "fs", "peer", "sdk", "proc", "ffi", "tmux", "shell", "attach",
    ]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub weight: Option<u64>,
    pub tier: Option<PluginTier>,
    pub wasm: Option<String>,
    pub entry: Option<String>,
    pub sdk: String,
    pub cli: Option<PluginCli>,
    pub api: Option<PluginApi>,
    pub description: Option<String>,
    pub author: Option<String>,
    pub hooks: Option<PluginHooks>,
    pub cron: Option<PluginCron>,
    pub module: Option<PluginModule>,
    pub transport: Option<PluginTransport>,
    pub engine: Option<PluginEngine>,
    pub target: Option<PluginTarget>,
    pub capability_namespaces: Option<Vec<String>>,
    pub capabilities: Option<Vec<String>>,
    pub capability_warnings: Vec<String>,
    pub dependencies: Option<PluginDependencies>,
    pub artifact: Option<PluginArtifact>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedPlugin {
    pub manifest: PluginManifest,
    pub dir: PathBuf,
    pub wasm_path: PathBuf,
    pub entry_path: Option<PathBuf>,
    pub kind: LoadedPluginKind,
    pub disabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadedPluginKind {
    Ts,
    Wasm,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginNameAndTier {
    pub name: String,
    pub tier: PluginTier,
}

#[derive(Debug, Clone)]
pub struct DiscoverPackagesOptions {
    pub scan_dirs: Vec<PathBuf>,
    pub disabled_plugins: Vec<String>,
    pub runtime_version: String,
    pub use_cache: bool,
}

impl Default for DiscoverPackagesOptions {
    fn default() -> Self {
        Self {
            scan_dirs: scan_dirs(),
            disabled_plugins: Vec::new(),
            runtime_version: runtime_sdk_version(),
            use_cache: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoverPackagesReport {
    pub plugins: Vec<LoadedPlugin>,
    pub warnings: Vec<String>,
}

static DISCOVER_CACHE: OnceLock<Mutex<Option<Vec<LoadedPlugin>>>> = OnceLock::new();
static MODULE_SYMBOL_CACHE: OnceLock<Mutex<BTreeMap<String, String>>> = OnceLock::new();

/// Parse and validate a `plugin.json` text.
///
/// # Errors
///
/// Returns maw-js-compatible validation messages for malformed manifests.
pub fn parse_manifest(json_text: &str, dir: &Path) -> Result<PluginManifest, String> {
    let raw: Value =
        serde_json::from_str(json_text).map_err(|_| "plugin.json: invalid JSON".to_owned())?;
    let object = parse_manifest_object(&raw)?;
    let name = parse_manifest_name(object)?;
    let version = parse_manifest_version(object)?;
    let weight = parse_manifest_weight(object)?;
    let sdk = parse_manifest_sdk(object)?;
    let wasm = parse_declared_manifest_file(object, "wasm", dir)?;
    let entry = parse_declared_manifest_file(object, "entry", dir)?;

    let capability_namespaces = parse_capability_namespaces(&raw)?;
    let extra_namespaces: Vec<&str> = capability_namespaces
        .as_ref()
        .map_or_else(Vec::new, |namespaces| {
            namespaces.iter().map(String::as_str).collect()
        });
    let capabilities = parse_capabilities(&raw, &extra_namespaces)?;
    let (capabilities, capability_warnings) = capabilities.map_or((None, Vec::new()), |parsed| {
        (Some(parsed.capabilities), parsed.warnings)
    });

    Ok(PluginManifest {
        name,
        version,
        weight,
        tier: parse_tier(&raw)?,
        wasm,
        entry,
        sdk,
        cli: parse_cli(&raw)?,
        api: parse_api(&raw)?,
        description: object
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_owned),
        author: object
            .get("author")
            .and_then(Value::as_str)
            .map(str::to_owned),
        hooks: parse_hooks(&raw)?,
        cron: parse_cron(&raw)?,
        module: parse_module(&raw)?,
        transport: parse_transport(&raw)?,
        engine: parse_engine(&raw)?,
        target: parse_target(&raw)?,
        capability_namespaces,
        capabilities,
        capability_warnings,
        dependencies: parse_dependencies(&raw)?,
        artifact: parse_artifact(&raw)?,
    })
}

/// Load and validate `plugin.json` from a plugin directory.
///
/// # Errors
///
/// Returns filesystem read errors or maw-js-compatible manifest validation messages when
/// `plugin.json` exists but cannot be loaded.
pub fn load_manifest_from_dir(dir: &Path) -> Result<Option<LoadedPlugin>, String> {
    let manifest_path = dir.join("plugin.json");
    if !manifest_path.exists() {
        return Ok(None);
    }
    let json_text = std::fs::read_to_string(&manifest_path)
        .map_err(|error| format!("plugin.json: failed to read: {error}"))?;
    let manifest = parse_manifest(&json_text, dir)?;
    let has_entry = manifest.entry.is_some();
    let has_artifact_js = manifest
        .artifact
        .as_ref()
        .is_some_and(|artifact| !artifact.path.is_empty());
    let effective_entry = manifest.entry.as_ref().or_else(|| {
        if has_artifact_js {
            manifest.artifact.as_ref().map(|artifact| &artifact.path)
        } else {
            None
        }
    });

    Ok(Some(LoadedPlugin {
        wasm_path: manifest
            .wasm
            .as_ref()
            .map_or_else(PathBuf::new, |wasm| resolve_dir_path(dir, wasm)),
        entry_path: effective_entry.map(|entry| resolve_dir_path(dir, entry)),
        kind: if has_entry || has_artifact_js {
            LoadedPluginKind::Ts
        } else {
            LoadedPluginKind::Wasm
        },
        disabled: false,
        dir: dir.to_path_buf(),
        manifest,
    }))
}

/// Scan plugin roots and return packages that pass maw-js Phase A registry gates.
///
/// # Errors
///
/// This function does not expose filesystem errors; unreadable roots, malformed manifests,
/// and refused plugins are skipped like maw-js `discoverPackages`.
#[must_use]
pub fn discover_packages(options: &DiscoverPackagesOptions) -> DiscoverPackagesReport {
    discover_packages_with_profile(options, |_| None)
}

/// Scan plugin roots with an injected active-profile resolver.
///
/// # Errors
///
/// This function does not expose filesystem errors; unreadable roots, malformed manifests,
/// and refused plugins are skipped like maw-js `discoverPackages`.
#[must_use]
pub fn discover_packages_with_profile<F>(
    options: &DiscoverPackagesOptions,
    resolve_active_profile_filter: F,
) -> DiscoverPackagesReport
where
    F: FnOnce(&[PluginNameAndTier]) -> Option<BTreeSet<String>>,
{
    if options.use_cache {
        if let Some(cached) = cached_discover_plugins() {
            return DiscoverPackagesReport {
                plugins: cached,
                warnings: Vec::new(),
            };
        }
    }

    let mut plugins = Vec::new();
    let mut warnings = Vec::new();
    let mut legacy_count = 0usize;

    for base_dir in &options.scan_dirs {
        let Ok(entries) = std::fs::read_dir(base_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_dir() && !file_type.is_symlink() {
                continue;
            }
            match discover_plugin_dir(&entry.path(), options) {
                PluginDiscovery::Loaded(loaded) => plugins.push(loaded),
                PluginDiscovery::Legacy(loaded) => {
                    legacy_count += 1;
                    plugins.push(loaded);
                }
                PluginDiscovery::Warning(warning) => warnings.push(warning),
                PluginDiscovery::Skip => {}
            }
        }
    }

    apply_weight_overrides(options.scan_dirs.first(), &mut plugins);
    plugins.sort_by_key(|plugin| plugin.manifest.weight.unwrap_or(50));

    let filter = resolve_active_profile_filter(
        &plugins
            .iter()
            .map(|plugin| PluginNameAndTier {
                name: plugin.manifest.name.clone(),
                tier: plugin.manifest.tier.unwrap_or(PluginTier::Core),
            })
            .collect::<Vec<_>>(),
    );
    if let Some(filter) = filter {
        plugins.retain(|plugin| filter.contains(&plugin.manifest.name));
    }

    if legacy_count > 0 {
        warnings.push(format!(
            "{legacy_count} legacy plugin{} loaded without artifact hash — build them to enforce integrity.",
            if legacy_count == 1 { "" } else { "s" }
        ));
    }

    if options.use_cache {
        cache_discover_plugins(plugins.clone());
    }

    DiscoverPackagesReport { plugins, warnings }
}

/// Clear registry discovery cache.
pub fn reset_discover_cache() {
    if let Ok(mut cache) = discover_cache().lock() {
        *cache = None;
    }
    if let Ok(mut cache) = module_symbol_cache().lock() {
        cache.clear();
    }
}

/// Import a whitelisted named symbol through an injected module loader.
///
/// This mirrors maw-js `importPluginSymbol` validation and caching, while leaving the
/// language-specific runtime module import to the caller.
///
/// # Errors
///
/// Returns maw-js-compatible errors for missing names, absent or disabled plugins,
/// missing module surfaces, unallowlisted symbols, module paths that escape the plugin
/// directory, loader failures, or runtime modules that omit the allowlisted export.
pub fn import_plugin_symbol<F>(
    plugin_name: &str,
    symbol_name: &str,
    plugins: &[LoadedPlugin],
    load_module_symbols: F,
) -> Result<String, String>
where
    F: FnOnce(&Path) -> Result<BTreeMap<String, String>, String>,
{
    if plugin_name.is_empty() {
        return Err("importPluginSymbol: pluginName is required".to_owned());
    }
    if symbol_name.is_empty() {
        return Err("importPluginSymbol: symbolName is required".to_owned());
    }

    let plugin = plugins
        .iter()
        .find(|plugin| plugin.manifest.name == plugin_name)
        .ok_or_else(|| format!("plugin '{plugin_name}' not found"))?;
    if plugin.disabled {
        return Err(format!("plugin '{plugin_name}' is disabled"));
    }
    let module_surface = plugin
        .manifest
        .module
        .as_ref()
        .ok_or_else(|| format!("plugin '{plugin_name}' does not declare a module surface"))?;
    if !module_surface
        .exports
        .iter()
        .any(|export| export == symbol_name)
    {
        return Err(format!(
            "plugin '{plugin_name}' does not export '{symbol_name}'"
        ));
    }

    let cache_key = format!("{}\0{plugin_name}\0{symbol_name}", plugin.dir.display());
    if let Some(value) = module_symbol_cache()
        .lock()
        .ok()
        .and_then(|cache| cache.get(&cache_key).cloned())
    {
        return Ok(value);
    }

    let module_path = resolve_plugin_module_path(plugin)?;
    let symbols = load_module_symbols(&module_path)?;
    let value = symbols.get(symbol_name).cloned().ok_or_else(|| {
        format!("plugin '{plugin_name}' module did not provide export '{symbol_name}'")
    })?;
    if let Ok(mut cache) = module_symbol_cache().lock() {
        cache.insert(cache_key, value.clone());
    }
    Ok(value)
}

/// Default plugin scan roots.
#[must_use]
pub fn scan_dirs() -> Vec<PathBuf> {
    std::env::var_os("MAW_PLUGINS_DIR").map_or_else(
        || {
            let home = std::env::var_os("MAW_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".maw")))
                .unwrap_or_else(|| PathBuf::from(".maw"));
            vec![home.join("plugins")]
        },
        |path| vec![PathBuf::from(path)],
    )
}

/// Runtime SDK version placeholder for registry gates.
#[must_use]
pub fn runtime_sdk_version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

/// Compute a `sha256:<hex>` digest for a file.
///
/// # Errors
///
/// Returns the filesystem read error text if the file cannot be read.
pub fn hash_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(&mut hex, "{byte:02x}");
    }
    Ok(format!("sha256:{hex}"))
}

/// True when the top-level plugin install path is a symlink/dev install.
#[must_use]
pub fn is_dev_mode_install(plugin_dir: &Path) -> bool {
    std::fs::symlink_metadata(plugin_dir).is_ok_and(|metadata| metadata.file_type().is_symlink())
}

/// Minimal maw-js-compatible semver range satisfaction.
#[must_use]
pub fn satisfies(version: &str, range: &str) -> bool {
    let Some(version) = parse_semver_core(version) else {
        return false;
    };
    let range = range.trim();
    if range == "*" {
        return true;
    }

    let (op, rest) = semver_operator(range);
    let Some(target) = parse_semver_core(rest) else {
        return false;
    };

    match op {
        Some("^") => caret_satisfies(version, target),
        Some("~") => {
            compare_semver(version, target).is_ge()
                && version.0 == target.0
                && version.1 == target.1
        }
        Some(">=") => compare_semver(version, target).is_ge(),
        Some("<=") => compare_semver(version, target).is_le(),
        Some(">") => compare_semver(version, target).is_gt(),
        Some("<") => compare_semver(version, target).is_lt(),
        _ => compare_semver(version, target).is_eq(),
    }
}

/// Format maw-js SDK mismatch warning text.
#[must_use]
pub fn format_sdk_mismatch_error(name: &str, manifest_sdk: &str, runtime_version: &str) -> String {
    [
        format!("✗ plugin '{name}' requires maw SDK {manifest_sdk}"),
        format!("  your maw: {runtime_version}  (SDK {runtime_version})"),
        String::new(),
        "  fix:".to_owned(),
        "    • maw update                                    (upgrade maw)".to_owned(),
        format!("    • maw plugin install {name}@<old-version>      (older compat release)"),
        "    • (manual) edit plugin.json \"sdk\" to accept this version and rebuild".to_owned(),
    ]
    .join("\n")
}

fn resolve_dir_path(dir: &Path, path: &str) -> PathBuf {
    let base = if dir.is_absolute() {
        dir.to_path_buf()
    } else {
        std::env::current_dir().map_or_else(|_| PathBuf::from(".").join(dir), |cwd| cwd.join(dir))
    };
    base.join(path)
}

fn discover_cache() -> &'static Mutex<Option<Vec<LoadedPlugin>>> {
    DISCOVER_CACHE.get_or_init(|| Mutex::new(None))
}

fn module_symbol_cache() -> &'static Mutex<BTreeMap<String, String>> {
    MODULE_SYMBOL_CACHE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn resolve_plugin_module_path(plugin: &LoadedPlugin) -> Result<PathBuf, String> {
    let module_path = plugin
        .manifest
        .module
        .as_ref()
        .map(|module| module.path.as_str())
        .ok_or_else(|| {
            format!(
                "plugin '{}' does not declare module.path",
                plugin.manifest.name
            )
        })?;
    let resolved = plugin.dir.join(module_path);
    let plugin_root = std::fs::canonicalize(&plugin.dir).map_err(|error| error.to_string())?;
    let real_path = std::fs::canonicalize(&resolved).map_err(|error| error.to_string())?;
    if real_path != plugin_root && !real_path.starts_with(&plugin_root) {
        return Err(format!(
            "plugin '{}' module.path escapes plugin dir: {module_path}",
            plugin.manifest.name
        ));
    }
    Ok(real_path)
}

fn cached_discover_plugins() -> Option<Vec<LoadedPlugin>> {
    discover_cache().lock().ok().and_then(|cache| cache.clone())
}

fn cache_discover_plugins(plugins: Vec<LoadedPlugin>) {
    if let Ok(mut cache) = discover_cache().lock() {
        *cache = Some(plugins);
    }
}

enum PluginDiscovery {
    Loaded(LoadedPlugin),
    Legacy(LoadedPlugin),
    Warning(String),
    Skip,
}

fn discover_plugin_dir(pkg_dir: &Path, options: &DiscoverPackagesOptions) -> PluginDiscovery {
    let Some(mut loaded) = load_manifest_from_dir(pkg_dir).ok().flatten() else {
        return PluginDiscovery::Skip;
    };
    let manifest = &loaded.manifest;

    if !satisfies(&options.runtime_version, &manifest.sdk) {
        return PluginDiscovery::Warning(format_sdk_mismatch_error(
            &manifest.name,
            &manifest.sdk,
            &options.runtime_version,
        ));
    }

    if let Some(warning) = artifact_refusal_warning(pkg_dir, manifest) {
        return PluginDiscovery::Warning(warning);
    }

    if options
        .disabled_plugins
        .iter()
        .any(|disabled| disabled == &loaded.manifest.name)
    {
        loaded.disabled = true;
    }

    let has_artifact = loaded.manifest.artifact.is_some();
    if has_artifact {
        PluginDiscovery::Loaded(loaded)
    } else {
        PluginDiscovery::Legacy(loaded)
    }
}

fn artifact_refusal_warning(pkg_dir: &Path, manifest: &PluginManifest) -> Option<String> {
    let artifact = manifest.artifact.as_ref()?;
    if is_dev_mode_install(pkg_dir) {
        return None;
    }
    let Some(expected_sha) = &artifact.sha256 else {
        return Some(format!(
            "plugin '{}' is unbuilt — run `maw plugin build` in {}",
            manifest.name,
            pkg_dir.display()
        ));
    };
    let artifact_path = pkg_dir.join(&artifact.path);
    if !artifact_path.exists() {
        return Some(format!(
            "plugin '{}' artifact missing: {}",
            manifest.name, artifact.path
        ));
    }
    match hash_file(&artifact_path) {
        Ok(observed) if observed == *expected_sha => None,
        Ok(observed) => Some(format!(
            "plugin '{}' artifact hash mismatch — refusing to load.\n  expected: {}\n  actual:   {}",
            manifest.name, expected_sha, observed
        )),
        Err(error) => Some(format!(
            "plugin '{}' artifact hash failed: {error}",
            manifest.name
        )),
    }
}

fn apply_weight_overrides(primary_plugin_dir: Option<&PathBuf>, plugins: &mut [LoadedPlugin]) {
    let Some(primary_plugin_dir) = primary_plugin_dir else {
        return;
    };
    let overrides_path = primary_plugin_dir.join(".overrides.json");
    let Ok(raw) = std::fs::read_to_string(overrides_path) else {
        return;
    };
    let Ok(overrides) = serde_json::from_str::<BTreeMap<String, u64>>(&raw) else {
        return;
    };
    for plugin in plugins {
        if let Some(weight) = overrides.get(&plugin.manifest.name) {
            plugin.manifest.weight = Some(*weight);
        }
    }
}

fn parse_semver_core(value: &str) -> Option<(u64, u64, u64)> {
    let trimmed = value.trim();
    let without_build = trimmed.split_once('+').map_or(trimmed, |(core, _)| core);
    let core = without_build
        .split_once('-')
        .map_or(without_build, |(core, _)| core);
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

fn semver_operator(range: &str) -> (Option<&'static str>, &str) {
    for op in [">=", "<=", "^", "~", ">", "<"] {
        if let Some(rest) = range.strip_prefix(op) {
            return (Some(op), rest);
        }
    }
    (None, range)
}

fn compare_semver(left: (u64, u64, u64), right: (u64, u64, u64)) -> std::cmp::Ordering {
    left.cmp(&right)
}

fn caret_satisfies(version: (u64, u64, u64), target: (u64, u64, u64)) -> bool {
    if compare_semver(version, target).is_lt() {
        return false;
    }
    if target.0 > 0 {
        return version.0 == target.0;
    }
    if target.1 > 0 {
        return version.0 == 0 && version.1 == target.1;
    }
    version.0 == 0 && version.1 == 0 && version.2 == target.2
}

fn parse_manifest_object(raw: &Value) -> Result<&Map<String, Value>, String> {
    raw.as_object()
        .ok_or_else(|| "plugin.json: must be a JSON object".to_owned())
}

fn parse_manifest_name(object: &Map<String, Value>) -> Result<String, String> {
    object
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| is_slug(name))
        .map(str::to_owned)
        .ok_or_else(|| {
            format!(
                "plugin.json: name must match /^[a-z0-9-]+$/ (got {})",
                manifest_field_for_error(object, "name")
            )
        })
}

fn parse_manifest_version(object: &Map<String, Value>) -> Result<String, String> {
    object
        .get("version")
        .and_then(Value::as_str)
        .filter(|version| is_semver(version))
        .map(str::to_owned)
        .ok_or_else(|| {
            format!(
                "plugin.json: version must be semver N.N.N (got {})",
                manifest_field_for_error(object, "version")
            )
        })
}

fn parse_manifest_weight(object: &Map<String, Value>) -> Result<Option<u64>, String> {
    let Some(value) = object.get("weight") else {
        return Ok(None);
    };
    let valid_weight = value
        .as_u64()
        .filter(|weight| *weight <= 99)
        .ok_or_else(weight_error)?;
    Ok(Some(valid_weight))
}

fn parse_manifest_sdk(object: &Map<String, Value>) -> Result<String, String> {
    object
        .get("sdk")
        .and_then(Value::as_str)
        .filter(|sdk| is_semver_range(sdk))
        .map(str::to_owned)
        .ok_or_else(|| {
            format!(
                "plugin.json: sdk must be a semver range (got {})",
                manifest_field_for_error(object, "sdk")
            )
        })
}

fn parse_declared_manifest_file(
    object: &Map<String, Value>,
    key: &str,
    dir: &Path,
) -> Result<Option<String>, String> {
    let Some(path) = object
        .get(key)
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty())
    else {
        return Ok(None);
    };
    let declared_path = dir.join(path);
    if declared_path.exists() {
        Ok(Some(path.to_owned()))
    } else {
        Err(format!(
            "plugin.json: {key} file not found: {}",
            declared_path.display()
        ))
    }
}

fn manifest_field_for_error(object: &Map<String, Value>, key: &str) -> String {
    object
        .get(key)
        .map_or("undefined".to_owned(), Value::to_string)
}

fn weight_error() -> String {
    "plugin.json: weight must be a number 0-99 (lower = runs first, default 50)".to_owned()
}

fn is_semver(value: &str) -> bool {
    let (core_and_pre, build_ok) = split_once_optional(value, '+');
    if !build_ok || core_and_pre.is_empty() {
        return false;
    }
    let (core, pre_ok) = split_once_optional(core_and_pre, '-');
    pre_ok && is_semver_core(core)
}

fn is_semver_range(value: &str) -> bool {
    if value == "*" {
        return true;
    }
    for op in [">=", "<=", "^", "~", ">", "<"] {
        if let Some(rest) = value.strip_prefix(op) {
            return is_semver(rest);
        }
    }
    is_semver(value)
}

fn split_once_optional(value: &str, separator: char) -> (&str, bool) {
    let mut parts = value.split(separator);
    let Some(first) = parts.next() else {
        return (value, false);
    };
    if parts.next().is_some_and(str::is_empty) {
        return (first, false);
    }
    if parts.next().is_some() {
        return (first, false);
    }
    (first, true)
}

fn is_semver_core(core: &str) -> bool {
    let mut parts = core.split('.');
    let Some(major) = parts.next() else {
        return false;
    };
    let Some(minor) = parts.next() else {
        return false;
    };
    let Some(patch) = parts.next() else {
        return false;
    };
    parts.next().is_none()
        && [major, minor, patch]
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}
