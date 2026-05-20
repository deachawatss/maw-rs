//! Minimal side-by-side maw-rs CLI dry-run surfaces.
//!
//! This crate intentionally starts with plan-only output so command parity can
//! be tested against maw-js parser contracts before host IO is wired.

use maw_auth::{
    sign_headers_v3_at, sign_request_v3, verify_request, FromVerifyDecision, Headers,
    VerifyRequestArgs,
};
use maw_auto_wake::{should_auto_wake, AutoWakeManifest, AutoWakeOptions, AutoWakeSite};
use maw_bind::{resolve_bind_host, BindConfig, BindHostResult};
use maw_bring::{parse_bring_args, BringAliasOptions, ParsedBringArgs};
use maw_calver::{compute_version, Channel, ComputeArgs, DateParts};
use maw_feed::{active_oracles_at, describe_activity, parse_line, FeedEvent};
use maw_fuzzy::{distance as fuzzy_distance, fuzzy_match};
use maw_hub::{
    load_workspace_configs, validate_workspace_config, WorkspaceConfig, WorkspaceConfigValidation,
};
use maw_identity::{canonical_node_identity, canonical_session_name, CanonicalSessionNameInput};
use maw_matcher::{
    normalize_target, resolve_by_name, resolve_session_target, resolve_worktree_target,
    ResolveOptions, ResolveResult,
};
use maw_peer::{
    classify_probe_error, format_probe_error, is_valid_maw_handshake, pick_probe_hint,
    probe_exit_code, resolve_peer_sources, safe_probe_host, DiscoveryResult, DiscoveryRow,
    NamedPeerConfig, PeerConfig, PeerSourceMode, PeerSourceResult, ProbeErrorCode,
    ProbeFailureInput, ProbeLastError, ProbeMawHandshake,
};
use maw_plugin_manifest::{
    discover_packages, import_plugin_symbol, invoke_plugin, load_manifest_from_dir, parse_manifest,
    DiscoverPackagesOptions, InvokeContext, InvokeResult, InvokeSource, LoadedPlugin,
    PluginInvokeRuntime, PluginManifest,
};
use maw_plugin_scaffold::{
    build_manifest_json, validate_plugin_name, PluginLanguage as ScaffoldLanguage,
};
use maw_policy::{default_active_group, weight_to_tier, DEFAULT_TIER, KNOWN_TIERS};
use maw_routing::{
    resolve_target as resolve_route_target, MawConfig as RouteConfig, NamedPeer as RouteNamedPeer,
    ResolveResult as RouteResult, Session as RouteSession, Window as RouteWindow,
};
use maw_split::{decide_split_policy, SplitPolicyDecision, SplitPolicyInput};
use maw_tmux::{
    mark_peer_targets_live, resolve_tmux_live_state, DiscoverLivePane, PeerTargetWithLive,
    TmuxLiveStateResult, TmuxPane,
};
use maw_transport::{
    classify_error, Transport, TransportFailureReason, TransportResult, TransportRouter,
    TransportTarget,
};
use maw_worktree::{
    resolve_worktree_window, Session as WorktreeSession, Window as WorktreeWindow,
    WorktreeWindowResolution,
};
use maw_xdg::{
    ensure_maw_core_paths, is_maw_xdg_enabled, is_valid_instance_name, maw_cache_dir,
    maw_cache_path, maw_config_dir, maw_config_path, maw_data_dir, maw_data_path,
    maw_runtime_home_dir, maw_state_dir, maw_state_path, MawCorePaths, MawXdgEnv,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliOutput {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Run the current maw-rs CLI parser/renderer over argv without process exit.
#[must_use]
pub fn run_cli(argv: &[String]) -> CliOutput {
    let Some(command) = argv.first().map(String::as_str) else {
        return usage_ok();
    };
    match command {
        "--help" | "-h" | "help" => usage_ok(),
        "auth" => run_auth_plan(&argv[1..]),
        "auto-wake" => run_auto_wake_plan(&argv[1..]),
        "hub" => run_hub_plan(&argv[1..]),
        "xdg" => run_xdg_plan(&argv[1..]),
        "plugin-scaffold" => run_plugin_scaffold_plan(&argv[1..]),
        "plugin-manifest" => run_plugin_manifest_plan(&argv[1..]),
        "bind-host" => run_bind_host_plan(&argv[1..]),
        "bring" | "b" => run_bring_plan(&argv[1..]),
        "feed" => run_feed_plan(&argv[1..]),
        "fuzzy" => run_fuzzy_plan(&argv[1..]),
        "resolve" => run_resolve_plan(&argv[1..]),
        "identity" => run_identity_plan(&argv[1..]),
        "normalize" => run_normalize_plan(&argv[1..]),
        "calver" => run_calver_plan(&argv[1..]),
        "worktree-window" => run_worktree_window_plan(&argv[1..]),
        "route" => run_route_plan(&argv[1..]),
        "discover" => run_discover_plan(&argv[1..]),
        "peer-sources" => run_peer_sources_plan(&argv[1..]),
        "peer-probe" => run_peer_probe_plan(&argv[1..]),
        "policy" | "plugin-policy" => run_policy_plan(&argv[1..]),
        "split-policy" => run_split_policy_plan(&argv[1..]),
        "transport" => run_transport_plan(&argv[1..]),
        _ => CliOutput {
            code: 2,
            stdout: String::new(),
            stderr: format!("unknown command: {command}\n{}", usage_text()),
        },
    }
}

#[allow(clippy::too_many_lines)]
fn run_auto_wake_plan(argv: &[String]) -> CliOutput {
    let mut plan_json = false;
    let mut target: Option<String> = None;
    let mut site = AutoWakeSite::View;
    let mut is_live = None;
    let mut is_fleet_known = None;
    let mut force = false;
    let mut no_wake = false;
    let mut is_canonical_target = false;
    let mut manifest_sources = Vec::new();
    let mut manifest_live = None;

    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--plan-json" => plan_json = true,
            "--site" => {
                let Some(value) = argv.get(index + 1) else {
                    return auto_wake_usage_error("auto-wake: missing --site value");
                };
                let Some(parsed) = parse_auto_wake_site(value) else {
                    return auto_wake_usage_error("auto-wake: invalid --site value");
                };
                site = parsed;
                index += 1;
            }
            arg if arg.starts_with("--site=") => {
                let Some(parsed) = parse_auto_wake_site(&arg["--site=".len()..]) else {
                    return auto_wake_usage_error("auto-wake: invalid --site value");
                };
                site = parsed;
            }
            "--live" => is_live = Some(true),
            "--not-live" => is_live = Some(false),
            "--fleet-known" => is_fleet_known = Some(true),
            "--unknown-fleet" => is_fleet_known = Some(false),
            "--wake" => force = true,
            "--no-wake" => no_wake = true,
            "--canonical-target" => is_canonical_target = true,
            "--manifest-source" => {
                let Some(value) = argv.get(index + 1) else {
                    return auto_wake_usage_error("auto-wake: missing --manifest-source value");
                };
                manifest_sources.push(value.to_owned());
                index += 1;
            }
            arg if arg.starts_with("--manifest-live=") => {
                let value = &arg["--manifest-live=".len()..];
                match parse_bool(value, "auto-wake: --manifest-live must be true or false") {
                    Ok(parsed) => manifest_live = Some(parsed),
                    Err(message) => return auto_wake_usage_error(&message),
                }
            }
            "--manifest-live" => {
                let Some(value) = argv.get(index + 1) else {
                    return auto_wake_usage_error("auto-wake: missing --manifest-live value");
                };
                match parse_bool(value, "auto-wake: --manifest-live must be true or false") {
                    Ok(parsed) => manifest_live = Some(parsed),
                    Err(message) => return auto_wake_usage_error(&message),
                }
                index += 1;
            }
            arg if arg.starts_with('-') => {
                return auto_wake_usage_error(&format!("auto-wake: unknown argument {arg}"));
            }
            value => {
                if target.is_some() {
                    return auto_wake_usage_error("auto-wake: target already provided");
                }
                target = Some(value.to_owned());
            }
        }
        index += 1;
    }

    let Some(target) = target else {
        return auto_wake_usage_error("auto-wake: missing target");
    };
    let manifest =
        (!manifest_sources.is_empty() || manifest_live.is_some()).then(|| AutoWakeManifest {
            name: target.clone(),
            sources: manifest_sources,
            is_live: manifest_live.unwrap_or(false),
        });
    let options = AutoWakeOptions {
        site,
        is_live,
        is_fleet_known,
        force,
        no_wake,
        is_canonical_target,
        manifest,
    };
    let decision = should_auto_wake(&target, options.clone());
    CliOutput {
        code: 0,
        stdout: if plan_json {
            render_auto_wake_plan_json(&target, &options, &decision)
        } else {
            format!(
                "auto-wake {} wake={} reason={}\n",
                target, decision.wake, decision.reason
            )
        },
        stderr: String::new(),
    }
}

fn parse_auto_wake_site(value: &str) -> Option<AutoWakeSite> {
    match value {
        "view" => Some(AutoWakeSite::View),
        "hey" => Some(AutoWakeSite::Hey),
        "api-send" => Some(AutoWakeSite::ApiSend),
        "api-wake" => Some(AutoWakeSite::ApiWake),
        "peek" => Some(AutoWakeSite::Peek),
        "bud" => Some(AutoWakeSite::Bud),
        "wake-cmd" => Some(AutoWakeSite::WakeCmd),
        _ => None,
    }
}

fn render_auto_wake_plan_json(
    target: &str,
    options: &AutoWakeOptions,
    decision: &maw_auto_wake::AutoWakeDecision,
) -> String {
    let manifest = options.manifest.as_ref().map_or_else(
        || "null".to_owned(),
        |manifest| {
            format!(
                "{{\"name\":{},\"sources\":{},\"isLive\":{}}}",
                json_string(&manifest.name),
                json_string_array(&manifest.sources),
                manifest.is_live
            )
        },
    );
    format!(
        "{{\"command\":\"auto-wake\",\"ok\":true,\"target\":{},\"site\":{},\"wake\":{},\"reason\":{},\"isLive\":{},\"isFleetKnown\":{},\"force\":{},\"noWake\":{},\"isCanonicalTarget\":{},\"manifest\":{}}}\n",
        json_string(target),
        json_string(auto_wake_site_name(options.site)),
        decision.wake,
        json_string(&decision.reason),
        json_opt_bool(options.is_live),
        json_opt_bool(options.is_fleet_known),
        options.force,
        options.no_wake,
        options.is_canonical_target,
        manifest
    )
}

fn auto_wake_site_name(site: AutoWakeSite) -> &'static str {
    match site {
        AutoWakeSite::View => "view",
        AutoWakeSite::Hey => "hey",
        AutoWakeSite::ApiSend => "api-send",
        AutoWakeSite::ApiWake => "api-wake",
        AutoWakeSite::Peek => "peek",
        AutoWakeSite::Bud => "bud",
        AutoWakeSite::WakeCmd => "wake-cmd",
    }
}

fn json_opt_bool(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "true",
        Some(false) => "false",
        None => "null",
    }
}

fn auto_wake_usage_error(message: &str) -> CliOutput {
    CliOutput {
        code: 2,
        stdout: String::new(),
        stderr: format!("{message}\n{}\n", auto_wake_usage()),
    }
}

fn auto_wake_usage() -> &'static str {
    "usage: maw-rs auto-wake <target> --site <view|hey|api-send|api-wake|peek|bud|wake-cmd> [--fleet-known|--unknown-fleet] [--live|--not-live] [--wake] [--no-wake] [--canonical-target] [--manifest-source <source>]... [--manifest-live <true|false>] [--plan-json]"
}

fn run_auth_plan(argv: &[String]) -> CliOutput {
    let action = match parse_auth_plan_args(argv) {
        Ok(action) => action,
        Err(message) => return auth_usage_error(&message),
    };
    match action {
        AuthPlanAction::SignV3 {
            plan_json,
            peer_key,
            from_address,
            method,
            path,
            timestamp,
            body,
        } => match sign_request_v3(
            &peer_key,
            &from_address,
            &method,
            &path,
            timestamp,
            body.as_deref().map(str::as_bytes),
        ) {
            Ok(signature) => {
                let headers = sign_headers_v3_at(
                    &peer_key,
                    &from_address,
                    &method,
                    &path,
                    body.as_deref().map(str::as_bytes),
                    timestamp,
                )
                .expect("sign_request_v3 succeeded with the same inputs");
                CliOutput {
                    code: 0,
                    stdout: if plan_json {
                        render_auth_sign_v3_json(
                            &method,
                            &path,
                            timestamp,
                            &from_address,
                            &signature.signature,
                            &signature.body_hash,
                            &headers,
                        )
                    } else {
                        format!("{}\n", signature.signature)
                    },
                    stderr: String::new(),
                }
            }
            Err(message) => auth_usage_error(&message),
        },
        AuthPlanAction::VerifyRequest {
            plan_json,
            method,
            path,
            timestamp,
            body,
            cached_pubkey,
            headers,
        } => {
            let decision = verify_request(&VerifyRequestArgs {
                method,
                path,
                headers: Headers::new(headers),
                body: body.map(std::string::String::into_bytes),
                cached_pubkey,
                now: timestamp,
            });
            CliOutput {
                code: 0,
                stdout: if plan_json {
                    render_auth_verify_json(&decision)
                } else {
                    format!("{}\n", decision.kind())
                },
                stderr: String::new(),
            }
        }
    }
}

enum AuthPlanAction {
    SignV3 {
        plan_json: bool,
        peer_key: String,
        from_address: String,
        method: String,
        path: String,
        timestamp: i64,
        body: Option<String>,
    },
    VerifyRequest {
        plan_json: bool,
        method: String,
        path: String,
        timestamp: i64,
        body: Option<String>,
        cached_pubkey: Option<String>,
        headers: Vec<(String, String)>,
    },
}

struct AuthCommonArgs {
    plan_json: bool,
    method: String,
    path: String,
    timestamp: i64,
    body: Option<String>,
}

fn parse_auth_plan_args(argv: &[String]) -> Result<AuthPlanAction, String> {
    let Some(kind) = argv.first().map(String::as_str) else {
        return Err("auth: expected sign-v3 or verify-request".to_owned());
    };
    match kind {
        "sign-v3" => parse_auth_sign_v3_args(&argv[1..]),
        "verify-request" => parse_auth_verify_args(&argv[1..]),
        other => Err(format!("auth: unknown subcommand {other}")),
    }
}

fn parse_auth_sign_v3_args(argv: &[String]) -> Result<AuthPlanAction, String> {
    let mut common = AuthCommonArgs {
        plan_json: false,
        method: "GET".to_owned(),
        path: "/".to_owned(),
        timestamp: 0,
        body: None,
    };
    let mut peer_key = None;
    let mut from_address = None;
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--plan-json" => common.plan_json = true,
            "--peer-key" => {
                peer_key = Some(take_auth_value(argv, index, "--peer-key")?);
                index += 1;
            }
            "--from" => {
                from_address = Some(take_auth_value(argv, index, "--from")?);
                index += 1;
            }
            "--method" => {
                common.method = take_auth_value(argv, index, "--method")?;
                index += 1;
            }
            "--path" => {
                common.path = take_auth_value(argv, index, "--path")?;
                index += 1;
            }
            "--now" => {
                let raw = take_auth_value(argv, index, "--now")?;
                common.timestamp = parse_i64_arg(&raw, "auth: --now")?;
                index += 1;
            }
            "--body" => {
                common.body = Some(take_auth_value(argv, index, "--body")?);
                index += 1;
            }
            other => return Err(format!("auth sign-v3: unknown argument {other}")),
        }
        index += 1;
    }
    Ok(AuthPlanAction::SignV3 {
        plan_json: common.plan_json,
        peer_key: peer_key.ok_or_else(|| "auth sign-v3: --peer-key is required".to_owned())?,
        from_address: from_address.ok_or_else(|| "auth sign-v3: --from is required".to_owned())?,
        method: common.method,
        path: common.path,
        timestamp: common.timestamp,
        body: common.body,
    })
}

fn parse_auth_verify_args(argv: &[String]) -> Result<AuthPlanAction, String> {
    let mut common = AuthCommonArgs {
        plan_json: false,
        method: "GET".to_owned(),
        path: "/".to_owned(),
        timestamp: 0,
        body: None,
    };
    let mut cached_pubkey = None;
    let mut headers = Vec::new();
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--plan-json" => common.plan_json = true,
            "--method" => {
                common.method = take_auth_value(argv, index, "--method")?;
                index += 1;
            }
            "--path" => {
                common.path = take_auth_value(argv, index, "--path")?;
                index += 1;
            }
            "--now" => {
                let raw = take_auth_value(argv, index, "--now")?;
                common.timestamp = parse_i64_arg(&raw, "auth: --now")?;
                index += 1;
            }
            "--body" => {
                common.body = Some(take_auth_value(argv, index, "--body")?);
                index += 1;
            }
            "--cached-pubkey" => {
                cached_pubkey = Some(take_auth_value(argv, index, "--cached-pubkey")?);
                index += 1;
            }
            "--header" => {
                let raw = take_auth_value(argv, index, "--header")?;
                let Some((name, value)) = raw.split_once('=') else {
                    return Err("auth verify-request: --header must be key=value".to_owned());
                };
                headers.push((name.to_owned(), value.to_owned()));
                index += 1;
            }
            other => return Err(format!("auth verify-request: unknown argument {other}")),
        }
        index += 1;
    }
    Ok(AuthPlanAction::VerifyRequest {
        plan_json: common.plan_json,
        method: common.method,
        path: common.path,
        timestamp: common.timestamp,
        body: common.body,
        cached_pubkey,
        headers,
    })
}

fn take_auth_value(argv: &[String], index: usize, name: &str) -> Result<String, String> {
    argv.get(index + 1)
        .cloned()
        .ok_or_else(|| format!("auth: missing {name} value"))
}

fn parse_i64_arg(value: &str, name: &str) -> Result<i64, String> {
    value
        .parse::<i64>()
        .map_err(|_| format!("{name} must be an integer"))
}

fn render_auth_sign_v3_json(
    method: &str,
    path: &str,
    timestamp: i64,
    from_address: &str,
    signature: &str,
    body_hash: &str,
    headers: &Headers,
) -> String {
    let header_map = headers.to_btree_map();
    let mut header_fields = Vec::new();
    for key in [
        "x-maw-auth-version",
        "x-maw-from",
        "x-maw-signature-v3",
        "x-maw-timestamp",
    ] {
        if let Some(value) = header_map.get(key) {
            let rendered_key = match key {
                "x-maw-auth-version" => "X-Maw-Auth-Version",
                "x-maw-from" => "X-Maw-From",
                "x-maw-signature-v3" => "X-Maw-Signature-V3",
                "x-maw-timestamp" => "X-Maw-Timestamp",
                _ => key,
            };
            header_fields.push(format!(
                "{}:{}",
                json_string(rendered_key),
                json_string(value)
            ));
        }
    }
    format!(
        "{{\"command\":\"auth\",\"kind\":\"sign-v3\",\"method\":{},\"path\":{},\"timestamp\":{timestamp},\"from\":{},\"signature\":{},\"bodyHash\":{},\"headers\":{{{}}}}}\n",
        json_string(method),
        json_string(path),
        json_string(from_address),
        json_string(signature),
        json_string(body_hash),
        header_fields.join(",")
    )
}

fn render_auth_verify_json(decision: &FromVerifyDecision) -> String {
    format!(
        "{{\"command\":\"auth\",\"kind\":\"verify-request\",\"decision\":{{{}}}}}\n",
        render_auth_decision_fields(decision).join(",")
    )
}

fn render_auth_decision_fields(decision: &FromVerifyDecision) -> Vec<String> {
    let mut fields = vec![format!("\"kind\":{}", json_string(decision.kind()))];
    match decision {
        FromVerifyDecision::AcceptLegacy { reason }
        | FromVerifyDecision::RefuseMalformed { reason } => {
            fields.push(format!("\"reason\":{}", json_string(reason)));
        }
        FromVerifyDecision::AcceptTofuRecord { reason, from }
        | FromVerifyDecision::AcceptVerified { reason, from }
        | FromVerifyDecision::RefuseMismatch { reason, from } => {
            fields.push(format!("\"reason\":{}", json_string(reason)));
            fields.push(format!("\"from\":{}", json_string(from)));
        }
        FromVerifyDecision::RefuseUnsigned { reason, from } => {
            fields.push(format!("\"reason\":{}", json_string(reason)));
            if let Some(from) = from {
                fields.push(format!("\"from\":{}", json_string(from)));
            }
        }
        FromVerifyDecision::RefuseSkew {
            reason,
            from,
            delta,
        } => {
            fields.push(format!("\"reason\":{}", json_string(reason)));
            fields.push(format!("\"from\":{}", json_string(from)));
            fields.push(format!("\"delta\":{delta}"));
        }
    }
    fields
}

fn auth_usage_error(message: &str) -> CliOutput {
    CliOutput {
        code: 2,
        stdout: String::new(),
        stderr: format!(
            "{message}\nusage: maw-rs auth sign-v3 --peer-key <key> --from <oracle:node> [--method <method>] [--path <path>] [--now <sec>] [--body <body>] [--plan-json]\n       maw-rs auth verify-request [--method <method>] [--path <path>] [--now <sec>] [--body <body>] [--cached-pubkey <key>] [--header <key=value>]... [--plan-json]\n"
        ),
    }
}

fn run_hub_plan(argv: &[String]) -> CliOutput {
    let action = match parse_hub_plan_args(argv) {
        Ok(action) => action,
        Err(message) => return hub_usage_error(&message),
    };
    match action {
        HubPlanAction::ValidateWorkspace {
            plan_json,
            id,
            hub_url,
            token,
            shared_agents,
        } => {
            let raw = serde_json::json!({
                "id": id,
                "hubUrl": hub_url,
                "token": token,
                "sharedAgents": shared_agents,
            });
            let validation = validate_workspace_config(&raw);
            CliOutput {
                code: 0,
                stdout: if plan_json {
                    render_hub_validate_json(&raw, &validation)
                } else if validation.ok() {
                    "ok\n".to_owned()
                } else {
                    format!("invalid: {}\n", validation.reason().unwrap_or("unknown"))
                },
                stderr: String::new(),
            }
        }
        HubPlanAction::LoadWorkspaces {
            plan_json,
            config_dir,
        } => match load_workspace_configs(&config_dir) {
            Ok(report) => CliOutput {
                code: 0,
                stdout: if plan_json {
                    render_hub_load_json(&report.configs, &report.warnings)
                } else {
                    format!(
                        "configs={} warnings={}\n",
                        report.configs.len(),
                        report.warnings.len()
                    )
                },
                stderr: String::new(),
            },
            Err(error) => CliOutput {
                code: 1,
                stdout: String::new(),
                stderr: format!("hub load-workspaces: {error}\n"),
            },
        },
    }
}

enum HubPlanAction {
    ValidateWorkspace {
        plan_json: bool,
        id: String,
        hub_url: String,
        token: String,
        shared_agents: Vec<String>,
    },
    LoadWorkspaces {
        plan_json: bool,
        config_dir: String,
    },
}

fn parse_hub_plan_args(argv: &[String]) -> Result<HubPlanAction, String> {
    let Some(kind) = argv.first().map(String::as_str) else {
        return Err("hub: expected validate-workspace or load-workspaces".to_owned());
    };
    match kind {
        "validate-workspace" => parse_hub_validate_args(&argv[1..]),
        "load-workspaces" => parse_hub_load_args(&argv[1..]),
        other => Err(format!("hub: unknown subcommand {other}")),
    }
}

fn parse_hub_validate_args(argv: &[String]) -> Result<HubPlanAction, String> {
    let mut plan_json = false;
    let mut id = String::new();
    let mut hub_url = String::new();
    let mut token = String::new();
    let mut shared_agents = Vec::new();
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--plan-json" => plan_json = true,
            "--id" => {
                id = take_hub_value(argv, index, "--id")?;
                index += 1;
            }
            "--hub-url" => {
                hub_url = take_hub_value(argv, index, "--hub-url")?;
                index += 1;
            }
            "--token" => {
                token = take_hub_value(argv, index, "--token")?;
                index += 1;
            }
            "--shared-agent" => {
                shared_agents.push(take_hub_value(argv, index, "--shared-agent")?);
                index += 1;
            }
            other => return Err(format!("hub validate-workspace: unknown argument {other}")),
        }
        index += 1;
    }
    Ok(HubPlanAction::ValidateWorkspace {
        plan_json,
        id,
        hub_url,
        token,
        shared_agents,
    })
}

fn parse_hub_load_args(argv: &[String]) -> Result<HubPlanAction, String> {
    let mut plan_json = false;
    let mut config_dir = None;
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--plan-json" => plan_json = true,
            "--config-dir" => {
                config_dir = Some(take_hub_value(argv, index, "--config-dir")?);
                index += 1;
            }
            other => return Err(format!("hub load-workspaces: unknown argument {other}")),
        }
        index += 1;
    }
    Ok(HubPlanAction::LoadWorkspaces {
        plan_json,
        config_dir: config_dir
            .ok_or_else(|| "hub load-workspaces: --config-dir is required".to_owned())?,
    })
}

fn take_hub_value(argv: &[String], index: usize, name: &str) -> Result<String, String> {
    argv.get(index + 1)
        .cloned()
        .ok_or_else(|| format!("hub: missing {name} value"))
}

fn render_hub_validate_json(
    raw: &serde_json::Value,
    validation: &WorkspaceConfigValidation,
) -> String {
    let reason = validation.reason().map_or("null".to_owned(), json_string);
    format!(
        "{{\"command\":\"hub\",\"kind\":\"validate-workspace\",\"input\":{},\"ok\":{},\"reason\":{reason}}}\n",
        raw,
        validation.ok()
    )
}

fn render_hub_load_json(configs: &[WorkspaceConfig], warnings: &[String]) -> String {
    let configs = configs
        .iter()
        .map(render_workspace_config_json)
        .collect::<Vec<_>>()
        .join(",");
    let warnings = json_string_array(warnings);
    format!(
        "{{\"command\":\"hub\",\"kind\":\"load-workspaces\",\"configs\":[{configs}],\"warnings\":{warnings}}}\n"
    )
}

fn render_workspace_config_json(config: &WorkspaceConfig) -> String {
    format!(
        "{{\"id\":{},\"hubUrl\":{},\"token\":{},\"sharedAgents\":{}}}",
        json_string(&config.id),
        json_string(&config.hub_url),
        json_string(&config.token),
        json_string_array(&config.shared_agents)
    )
}

fn hub_usage_error(message: &str) -> CliOutput {
    CliOutput {
        code: 2,
        stdout: String::new(),
        stderr: format!(
            "{message}\nusage: maw-rs hub validate-workspace [--id <id>] [--hub-url <ws-url>] [--token <token>] [--shared-agent <agent>]... [--plan-json]\n       maw-rs hub load-workspaces --config-dir <dir> [--plan-json]\n"
        ),
    }
}

fn run_xdg_plan(argv: &[String]) -> CliOutput {
    let action = match parse_xdg_plan_args(argv) {
        Ok(action) => action,
        Err(message) => return xdg_usage_error(&message),
    };
    match action {
        XdgPlanAction::Paths { plan_json, env } => {
            let paths = XdgResolvedPaths::from_env(&env);
            CliOutput {
                code: 0,
                stdout: if plan_json {
                    render_xdg_paths_json(&paths)
                } else {
                    format!("{}\n", paths.runtime_home)
                },
                stderr: String::new(),
            }
        }
        XdgPlanAction::CorePaths { plan_json, env } => match ensure_maw_core_paths(&env) {
            Ok(paths) => CliOutput {
                code: 0,
                stdout: if plan_json {
                    render_xdg_core_paths_json(&paths)
                } else {
                    format!("{}\n", paths.runtime_home.display())
                },
                stderr: String::new(),
            },
            Err(error) => CliOutput {
                code: 1,
                stdout: String::new(),
                stderr: format!("xdg core-paths: {error}\n"),
            },
        },
        XdgPlanAction::ValidateInstance { plan_json, name } => {
            let valid = is_valid_instance_name(&name);
            CliOutput {
                code: 0,
                stdout: if plan_json {
                    format!(
                        "{{\"command\":\"xdg\",\"kind\":\"validate-instance\",\"name\":{},\"valid\":{valid}}}\n",
                        json_string(&name)
                    )
                } else {
                    format!("{valid}\n")
                },
                stderr: String::new(),
            }
        }
    }
}

enum XdgPlanAction {
    Paths { plan_json: bool, env: MawXdgEnv },
    CorePaths { plan_json: bool, env: MawXdgEnv },
    ValidateInstance { plan_json: bool, name: String },
}

struct XdgCliEnvArgs {
    plan_json: bool,
    home: String,
    vars: Vec<(String, String)>,
}

struct XdgResolvedPaths {
    xdg_enabled: bool,
    runtime_home: String,
    data_dir: String,
    state_dir: String,
    cache_dir: String,
    config_dir: String,
    data_path: String,
    state_path: String,
    cache_path: String,
    config_path: String,
}

impl XdgResolvedPaths {
    fn from_env(env: &MawXdgEnv) -> Self {
        Self {
            xdg_enabled: is_maw_xdg_enabled(env),
            runtime_home: path_string(maw_runtime_home_dir(env)),
            data_dir: path_string(maw_data_dir(env)),
            state_dir: path_string(maw_state_dir(env)),
            cache_dir: path_string(maw_cache_dir(env)),
            config_dir: path_string(maw_config_dir(env)),
            data_path: path_string(maw_data_path(env, &["plugins"])),
            state_path: path_string(maw_state_path(env, &["peers.json"])),
            cache_path: path_string(maw_cache_path(env, &["registry-cache.json"])),
            config_path: path_string(maw_config_path(env, &["maw.config.json"])),
        }
    }
}

fn parse_xdg_plan_args(argv: &[String]) -> Result<XdgPlanAction, String> {
    let Some(kind) = argv.first().map(String::as_str) else {
        return Err("xdg: expected paths, core-paths, or validate-instance".to_owned());
    };
    match kind {
        "paths" => {
            let parsed = parse_xdg_env_args(&argv[1..])?;
            Ok(XdgPlanAction::Paths {
                plan_json: parsed.plan_json,
                env: MawXdgEnv::with_vars(parsed.home, parsed.vars),
            })
        }
        "core-paths" => {
            let parsed = parse_xdg_env_args(&argv[1..])?;
            Ok(XdgPlanAction::CorePaths {
                plan_json: parsed.plan_json,
                env: MawXdgEnv::with_vars(parsed.home, parsed.vars),
            })
        }
        "validate-instance" => parse_xdg_validate_instance_args(&argv[1..]),
        other => Err(format!("xdg: unknown subcommand {other}")),
    }
}

fn parse_xdg_env_args(argv: &[String]) -> Result<XdgCliEnvArgs, String> {
    let mut plan_json = false;
    let mut home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_owned());
    let mut vars = Vec::new();
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--plan-json" => plan_json = true,
            "--home" => {
                home = take_xdg_value(argv, index, "--home")?;
                index += 1;
            }
            "--env" => {
                let raw = take_xdg_value(argv, index, "--env")?;
                let Some((key, value)) = raw.split_once('=') else {
                    return Err("xdg: --env must be KEY=VALUE".to_owned());
                };
                vars.push((key.to_owned(), value.to_owned()));
                index += 1;
            }
            other => return Err(format!("xdg: unknown argument {other}")),
        }
        index += 1;
    }
    Ok(XdgCliEnvArgs {
        plan_json,
        home,
        vars,
    })
}

fn parse_xdg_validate_instance_args(argv: &[String]) -> Result<XdgPlanAction, String> {
    let mut plan_json = false;
    let mut name = None;
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--plan-json" => plan_json = true,
            "--name" => {
                name = Some(take_xdg_value(argv, index, "--name")?);
                index += 1;
            }
            other => return Err(format!("xdg validate-instance: unknown argument {other}")),
        }
        index += 1;
    }
    Ok(XdgPlanAction::ValidateInstance {
        plan_json,
        name: name.ok_or_else(|| "xdg validate-instance: --name is required".to_owned())?,
    })
}

fn take_xdg_value(argv: &[String], index: usize, name: &str) -> Result<String, String> {
    argv.get(index + 1)
        .cloned()
        .ok_or_else(|| format!("xdg: missing {name} value"))
}

fn render_xdg_paths_json(paths: &XdgResolvedPaths) -> String {
    format!(
        "{{\"command\":\"xdg\",\"kind\":\"paths\",\"xdgEnabled\":{},\"runtimeHome\":{},\"dataDir\":{},\"stateDir\":{},\"cacheDir\":{},\"configDir\":{},\"dataPath\":{},\"statePath\":{},\"cachePath\":{},\"configPath\":{}}}\n",
        paths.xdg_enabled,
        json_string(&paths.runtime_home),
        json_string(&paths.data_dir),
        json_string(&paths.state_dir),
        json_string(&paths.cache_dir),
        json_string(&paths.config_dir),
        json_string(&paths.data_path),
        json_string(&paths.state_path),
        json_string(&paths.cache_path),
        json_string(&paths.config_path)
    )
}

fn render_xdg_core_paths_json(paths: &MawCorePaths) -> String {
    format!(
        "{{\"command\":\"xdg\",\"kind\":\"core-paths\",\"runtimeHome\":{},\"configDir\":{},\"fleetDir\":{},\"configFile\":{}}}\n",
        json_string(&path_string(&paths.runtime_home)),
        json_string(&path_string(&paths.config_dir)),
        json_string(&path_string(&paths.fleet_dir)),
        json_string(&path_string(&paths.config_file))
    )
}

fn path_string(path: impl AsRef<std::path::Path>) -> String {
    path.as_ref().to_string_lossy().into_owned()
}

fn xdg_usage_error(message: &str) -> CliOutput {
    CliOutput {
        code: 2,
        stdout: String::new(),
        stderr: format!(
            "{message}\nusage: maw-rs xdg paths [--home <dir>] [--env <KEY=VALUE>]... [--plan-json]\n       maw-rs xdg core-paths [--home <dir>] [--env <KEY=VALUE>]... [--plan-json]\n       maw-rs xdg validate-instance --name <name> [--plan-json]\n"
        ),
    }
}

fn run_plugin_scaffold_plan(argv: &[String]) -> CliOutput {
    let action = match parse_plugin_scaffold_args(argv) {
        Ok(action) => action,
        Err(message) => return plugin_scaffold_usage_error(&message),
    };
    match action {
        PluginScaffoldAction::ValidateName { plan_json, name } => {
            let error = validate_plugin_name(&name);
            let valid = error.is_none();
            CliOutput {
                code: 0,
                stdout: if plan_json {
                    let error_json = error.map_or("null".to_owned(), |error| json_string(&error));
                    format!(
                        "{{\"command\":\"plugin-scaffold\",\"kind\":\"validate-name\",\"name\":{},\"valid\":{valid},\"error\":{error_json}}}\n",
                        json_string(&name)
                    )
                } else if valid {
                    "valid\n".to_owned()
                } else {
                    format!("{}\n", error.expect("invalid name has error"))
                },
                stderr: String::new(),
            }
        }
        PluginScaffoldAction::Manifest {
            plan_json,
            name,
            language,
        } => {
            let manifest_text = build_manifest_json(&name, language);
            let manifest: serde_json::Value = serde_json::from_str(&manifest_text)
                .expect("maw-plugin-scaffold emits valid manifest JSON");
            let language_name = match language {
                ScaffoldLanguage::Rust => "rust",
                ScaffoldLanguage::AssemblyScript => "assemblyscript",
            };
            CliOutput {
                code: 0,
                stdout: if plan_json {
                    format!(
                        "{{\"command\":\"plugin-scaffold\",\"kind\":\"manifest\",\"language\":{},\"manifest\":{manifest}}}\n",
                        json_string(language_name)
                    )
                } else {
                    manifest_text
                },
                stderr: String::new(),
            }
        }
    }
}

enum PluginScaffoldAction {
    ValidateName {
        plan_json: bool,
        name: String,
    },
    Manifest {
        plan_json: bool,
        name: String,
        language: ScaffoldLanguage,
    },
}

fn parse_plugin_scaffold_args(argv: &[String]) -> Result<PluginScaffoldAction, String> {
    let Some(kind) = argv.first().map(String::as_str) else {
        return Err("plugin-scaffold: expected validate-name or manifest".to_owned());
    };
    match kind {
        "validate-name" => parse_plugin_scaffold_validate_args(&argv[1..]),
        "manifest" => parse_plugin_scaffold_manifest_args(&argv[1..]),
        other => Err(format!("plugin-scaffold: unknown subcommand {other}")),
    }
}

fn parse_plugin_scaffold_validate_args(argv: &[String]) -> Result<PluginScaffoldAction, String> {
    let mut plan_json = false;
    let mut name = None;
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--plan-json" => plan_json = true,
            "--name" => {
                name = Some(take_plugin_scaffold_value(argv, index, "--name")?);
                index += 1;
            }
            other => {
                return Err(format!(
                    "plugin-scaffold validate-name: unknown argument {other}"
                ))
            }
        }
        index += 1;
    }
    Ok(PluginScaffoldAction::ValidateName {
        plan_json,
        name: name.ok_or_else(|| "plugin-scaffold validate-name: --name is required".to_owned())?,
    })
}

fn parse_plugin_scaffold_manifest_args(argv: &[String]) -> Result<PluginScaffoldAction, String> {
    let mut plan_json = false;
    let mut name = None;
    let mut rust = false;
    let mut assembly_script = false;
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--plan-json" => plan_json = true,
            "--name" => {
                name = Some(take_plugin_scaffold_value(argv, index, "--name")?);
                index += 1;
            }
            "--rust" => rust = true,
            "--as" => assembly_script = true,
            other => {
                return Err(format!(
                    "plugin-scaffold manifest: unknown argument {other}"
                ))
            }
        }
        index += 1;
    }
    if !rust && !assembly_script {
        return Err("plugin-scaffold manifest: Specify either --rust or --as".to_owned());
    }
    if rust && assembly_script {
        return Err("plugin-scaffold manifest: Specify --rust or --as, not both".to_owned());
    }
    let name = name.ok_or_else(|| "plugin-scaffold manifest: --name is required".to_owned())?;
    if let Some(error) = validate_plugin_name(&name) {
        return Err(format!(
            "plugin-scaffold manifest: Invalid plugin name: {error}"
        ));
    }
    Ok(PluginScaffoldAction::Manifest {
        plan_json,
        name,
        language: if rust {
            ScaffoldLanguage::Rust
        } else {
            ScaffoldLanguage::AssemblyScript
        },
    })
}

fn take_plugin_scaffold_value(argv: &[String], index: usize, name: &str) -> Result<String, String> {
    argv.get(index + 1)
        .cloned()
        .ok_or_else(|| format!("plugin-scaffold: missing {name} value"))
}

fn plugin_scaffold_usage_error(message: &str) -> CliOutput {
    CliOutput {
        code: 2,
        stdout: String::new(),
        stderr: format!(
            "{message}\nusage: maw-rs plugin-scaffold validate-name --name <name> [--plan-json]\n       maw-rs plugin-scaffold manifest --name <name> (--rust|--as) [--plan-json]\n"
        ),
    }
}

fn run_plugin_manifest_plan(argv: &[String]) -> CliOutput {
    let action = match parse_plugin_manifest_args(argv) {
        Ok(action) => action,
        Err(message) => return plugin_manifest_usage_error(&message),
    };
    match action {
        PluginManifestAction::Parse {
            plan_json,
            dir,
            json_text,
        } => match parse_manifest(&json_text, &dir) {
            Ok(manifest) => CliOutput {
                code: 0,
                stdout: if plan_json {
                    format!(
                        "{{\"command\":\"plugin-manifest\",\"kind\":\"parse\",\"dir\":{},\"manifest\":{}}}\n",
                        json_string(&path_string(&dir)),
                        render_plugin_manifest_json(&manifest)
                    )
                } else {
                    format!("{}\n", manifest.name)
                },
                stderr: String::new(),
            },
            Err(message) => plugin_manifest_usage_error(&message),
        },
        PluginManifestAction::Load { plan_json, dir } => match load_manifest_from_dir(&dir) {
            Ok(plugin) => CliOutput {
                code: 0,
                stdout: if plan_json {
                    let plugin_json = plugin
                        .as_ref()
                        .map_or_else(|| "null".to_owned(), render_loaded_plugin_json);
                    format!(
                        "{{\"command\":\"plugin-manifest\",\"kind\":\"load\",\"dir\":{},\"present\":{},\"plugin\":{plugin_json}}}\n",
                        json_string(&path_string(&dir)),
                        plugin.is_some()
                    )
                } else {
                    plugin.map_or_else(
                        || "missing\n".to_owned(),
                        |plugin| format!("{} {}\n", plugin.kind.as_str(), plugin.manifest.name),
                    )
                },
                stderr: String::new(),
            },
            Err(message) => plugin_manifest_usage_error(&message),
        },
        PluginManifestAction::Discover { plan_json, options } => {
            let report = discover_packages(&options);
            CliOutput {
                code: 0,
                stdout: if plan_json {
                    render_plugin_discover_json(&options, &report.plugins, &report.warnings)
                } else {
                    let mut names = report
                        .plugins
                        .iter()
                        .map(|plugin| plugin.manifest.name.as_str())
                        .collect::<Vec<_>>()
                        .join("\n");
                    names.push('\n');
                    names
                },
                stderr: String::new(),
            }
        }
        PluginManifestAction::ImportSymbol {
            plan_json,
            options,
            plugin,
            symbol,
            module_symbols,
        } => run_plugin_manifest_import_symbol_plan(
            plan_json,
            &options,
            &plugin,
            &symbol,
            &module_symbols,
        ),
        PluginManifestAction::Invoke {
            plan_json,
            options,
            plugin,
            source,
            args,
            fake_ts_output,
            fake_wasm_output,
        } => run_plugin_manifest_invoke_plan(
            plan_json,
            &options,
            &plugin,
            source,
            args,
            fake_ts_output,
            fake_wasm_output,
        ),
    }
}

fn run_plugin_manifest_import_symbol_plan(
    plan_json: bool,
    options: &DiscoverPackagesOptions,
    plugin: &str,
    symbol: &str,
    module_symbols: &BTreeMap<String, String>,
) -> CliOutput {
    let report = discover_packages(options);
    let mut module_path = None;
    match import_plugin_symbol(plugin, symbol, &report.plugins, |path| {
        module_path = Some(path.to_path_buf());
        Ok(module_symbols.clone())
    }) {
        Ok(value) => CliOutput {
            code: 0,
            stdout: if plan_json {
                render_plugin_import_symbol_json(
                    plugin,
                    symbol,
                    &value,
                    module_path.as_deref(),
                    &report.warnings,
                )
            } else {
                format!("{value}\n")
            },
            stderr: String::new(),
        },
        Err(message) => plugin_manifest_usage_error(&message),
    }
}

fn run_plugin_manifest_invoke_plan(
    plan_json: bool,
    options: &DiscoverPackagesOptions,
    plugin_name: &str,
    source: InvokeSource,
    args: Vec<String>,
    fake_ts_output: Option<String>,
    fake_wasm_output: Option<String>,
) -> CliOutput {
    let report = discover_packages(options);
    let Some(plugin) = report
        .plugins
        .iter()
        .find(|plugin| plugin.manifest.name == plugin_name)
    else {
        return plugin_manifest_usage_error(&format!("plugin '{plugin_name}' not found"));
    };
    if plugin.disabled {
        return plugin_manifest_usage_error(&format!("plugin '{plugin_name}' is disabled"));
    }
    let ctx = InvokeContext { source, args };
    let mut runtime = PlanInvokeRuntime::new(fake_ts_output, fake_wasm_output);
    let result = invoke_plugin(plugin, &ctx, &mut runtime);
    CliOutput {
        code: 0,
        stdout: if plan_json {
            render_plugin_invoke_json(plugin_name, &ctx, &result, &runtime, &report.warnings)
        } else if result.ok {
            result
                .output
                .map_or_else(|| "ok\n".to_owned(), |output| format!("{output}\n"))
        } else {
            format!("{}\n", result.error.unwrap_or_else(|| "error".to_owned()))
        },
        stderr: String::new(),
    }
}

struct PlanInvokeRuntime {
    ts_calls: usize,
    wasm_calls: usize,
    last_wasm_bytes_len: usize,
    ts_result: InvokeResult,
    wasm_result: InvokeResult,
}

impl PlanInvokeRuntime {
    fn new(fake_ts_output: Option<String>, fake_wasm_output: Option<String>) -> Self {
        Self {
            ts_calls: 0,
            wasm_calls: 0,
            last_wasm_bytes_len: 0,
            ts_result: fake_ts_output.map_or_else(InvokeResult::ok, InvokeResult::output),
            wasm_result: fake_wasm_output.map_or_else(InvokeResult::ok, InvokeResult::output),
        }
    }
}

impl PluginInvokeRuntime for PlanInvokeRuntime {
    fn invoke_ts(&mut self, _plugin: &LoadedPlugin, _ctx: &InvokeContext) -> InvokeResult {
        self.ts_calls += 1;
        self.ts_result.clone()
    }

    fn invoke_wasm(
        &mut self,
        _plugin: &LoadedPlugin,
        _ctx: &InvokeContext,
        wasm_bytes: &[u8],
    ) -> InvokeResult {
        self.wasm_calls += 1;
        self.last_wasm_bytes_len = wasm_bytes.len();
        self.wasm_result.clone()
    }
}

enum PluginManifestAction {
    Parse {
        plan_json: bool,
        dir: std::path::PathBuf,
        json_text: String,
    },
    Load {
        plan_json: bool,
        dir: std::path::PathBuf,
    },
    Discover {
        plan_json: bool,
        options: DiscoverPackagesOptions,
    },
    ImportSymbol {
        plan_json: bool,
        options: DiscoverPackagesOptions,
        plugin: String,
        symbol: String,
        module_symbols: BTreeMap<String, String>,
    },
    Invoke {
        plan_json: bool,
        options: DiscoverPackagesOptions,
        plugin: String,
        source: InvokeSource,
        args: Vec<String>,
        fake_ts_output: Option<String>,
        fake_wasm_output: Option<String>,
    },
}

fn parse_plugin_manifest_args(argv: &[String]) -> Result<PluginManifestAction, String> {
    let Some(kind) = argv.first().map(String::as_str) else {
        return Err("plugin-manifest: expected parse or load".to_owned());
    };
    match kind {
        "parse" => parse_plugin_manifest_parse_args(&argv[1..]),
        "load" => parse_plugin_manifest_load_args(&argv[1..]),
        "discover" => parse_plugin_manifest_discover_args(&argv[1..]),
        "import-symbol" => parse_plugin_manifest_import_symbol_args(&argv[1..]),
        "invoke" => parse_plugin_manifest_invoke_args(&argv[1..]),
        other => Err(format!("plugin-manifest: unknown subcommand {other}")),
    }
}

fn parse_plugin_manifest_parse_args(argv: &[String]) -> Result<PluginManifestAction, String> {
    let mut plan_json = false;
    let mut dir = std::path::PathBuf::from(".");
    let mut json_text = None;
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--plan-json" => plan_json = true,
            "--dir" => {
                dir = take_plugin_manifest_path(argv, index, "--dir")?;
                index += 1;
            }
            "--json" => {
                json_text = Some(take_plugin_manifest_value(argv, index, "--json")?);
                index += 1;
            }
            other => return Err(format!("plugin-manifest parse: unknown argument {other}")),
        }
        index += 1;
    }
    Ok(PluginManifestAction::Parse {
        plan_json,
        dir,
        json_text: json_text
            .ok_or_else(|| "plugin-manifest parse: --json is required".to_owned())?,
    })
}

fn parse_plugin_manifest_load_args(argv: &[String]) -> Result<PluginManifestAction, String> {
    let mut plan_json = false;
    let mut dir = std::path::PathBuf::from(".");
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--plan-json" => plan_json = true,
            "--dir" => {
                dir = take_plugin_manifest_path(argv, index, "--dir")?;
                index += 1;
            }
            other => return Err(format!("plugin-manifest load: unknown argument {other}")),
        }
        index += 1;
    }
    Ok(PluginManifestAction::Load { plan_json, dir })
}

fn parse_plugin_manifest_discover_args(argv: &[String]) -> Result<PluginManifestAction, String> {
    let (plan_json, options, _) = parse_plugin_manifest_registry_args(argv, false)?;
    Ok(PluginManifestAction::Discover { plan_json, options })
}

fn parse_plugin_manifest_import_symbol_args(
    argv: &[String],
) -> Result<PluginManifestAction, String> {
    let (plan_json, options, import) = parse_plugin_manifest_registry_args(argv, true)?;
    let import = import.expect("import parser requested import args");
    Ok(PluginManifestAction::ImportSymbol {
        plan_json,
        options,
        plugin: import.plugin,
        symbol: import.symbol,
        module_symbols: import.module_symbols,
    })
}

fn parse_plugin_manifest_invoke_args(argv: &[String]) -> Result<PluginManifestAction, String> {
    let mut plan_json = false;
    let mut scan_dirs = Vec::new();
    let mut disabled_plugins = Vec::new();
    let mut runtime_version = "1.0.0".to_owned();
    let mut use_cache = false;
    let mut plugin = None;
    let mut source = InvokeSource::Cli;
    let mut invoke_args = Vec::new();
    let mut fake_ts_output = None;
    let mut fake_wasm_output = None;
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--plan-json" => plan_json = true,
            "--scan-dir" => {
                scan_dirs.push(take_plugin_manifest_path(argv, index, "--scan-dir")?);
                index += 1;
            }
            "--disabled" => {
                disabled_plugins.push(take_plugin_manifest_value(argv, index, "--disabled")?);
                index += 1;
            }
            "--runtime-version" => {
                runtime_version = take_plugin_manifest_value(argv, index, "--runtime-version")?;
                index += 1;
            }
            "--use-cache" => use_cache = true,
            "--plugin" => {
                plugin = Some(take_plugin_manifest_value(argv, index, "--plugin")?);
                index += 1;
            }
            "--source" => {
                source = parse_plugin_manifest_invoke_source(&take_plugin_manifest_value(
                    argv, index, "--source",
                )?)?;
                index += 1;
            }
            "--arg" => {
                invoke_args.push(take_plugin_manifest_value(argv, index, "--arg")?);
                index += 1;
            }
            "--fake-ts-output" => {
                fake_ts_output = Some(take_plugin_manifest_value(argv, index, "--fake-ts-output")?);
                index += 1;
            }
            "--fake-wasm-output" => {
                fake_wasm_output = Some(take_plugin_manifest_value(
                    argv,
                    index,
                    "--fake-wasm-output",
                )?);
                index += 1;
            }
            other => return Err(format!("plugin-manifest invoke: unknown argument {other}")),
        }
        index += 1;
    }
    if scan_dirs.is_empty() {
        return Err("plugin-manifest invoke: --scan-dir is required".to_owned());
    }
    Ok(PluginManifestAction::Invoke {
        plan_json,
        options: DiscoverPackagesOptions {
            scan_dirs,
            disabled_plugins,
            runtime_version,
            use_cache,
        },
        plugin: plugin.ok_or_else(|| "plugin-manifest invoke: --plugin is required".to_owned())?,
        source,
        args: invoke_args,
        fake_ts_output,
        fake_wasm_output,
    })
}

fn parse_plugin_manifest_invoke_source(value: &str) -> Result<InvokeSource, String> {
    match value {
        "cli" => Ok(InvokeSource::Cli),
        "api" => Ok(InvokeSource::Api),
        "peer" => Ok(InvokeSource::Peer),
        other => Err(format!("plugin-manifest invoke: unknown --source {other}")),
    }
}

struct PluginManifestImportArgs {
    plugin: String,
    symbol: String,
    module_symbols: BTreeMap<String, String>,
}

fn parse_plugin_manifest_registry_args(
    argv: &[String],
    include_import_args: bool,
) -> Result<
    (
        bool,
        DiscoverPackagesOptions,
        Option<PluginManifestImportArgs>,
    ),
    String,
> {
    let mut plan_json = false;
    let mut scan_dirs = Vec::new();
    let mut disabled_plugins = Vec::new();
    let mut runtime_version = "1.0.0".to_owned();
    let mut use_cache = false;
    let mut plugin = None;
    let mut symbol = None;
    let mut module_symbols = BTreeMap::new();
    let command = if include_import_args {
        "plugin-manifest import-symbol"
    } else {
        "plugin-manifest discover"
    };
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--plan-json" => plan_json = true,
            "--scan-dir" => {
                scan_dirs.push(take_plugin_manifest_path(argv, index, "--scan-dir")?);
                index += 1;
            }
            "--disabled" => {
                disabled_plugins.push(take_plugin_manifest_value(argv, index, "--disabled")?);
                index += 1;
            }
            "--runtime-version" => {
                runtime_version = take_plugin_manifest_value(argv, index, "--runtime-version")?;
                index += 1;
            }
            "--use-cache" => use_cache = true,
            "--plugin" if include_import_args => {
                plugin = Some(take_plugin_manifest_value(argv, index, "--plugin")?);
                index += 1;
            }
            "--symbol" if include_import_args => {
                symbol = Some(take_plugin_manifest_value(argv, index, "--symbol")?);
                index += 1;
            }
            "--module-symbol" if include_import_args => {
                let raw = take_plugin_manifest_value(argv, index, "--module-symbol")?;
                let Some((name, value)) = raw.split_once('=') else {
                    return Err(
                        "plugin-manifest import-symbol: --module-symbol must be name=value"
                            .to_owned(),
                    );
                };
                module_symbols.insert(name.to_owned(), value.to_owned());
                index += 1;
            }
            other => return Err(format!("{command}: unknown argument {other}")),
        }
        index += 1;
    }
    if scan_dirs.is_empty() {
        return Err(format!("{command}: --scan-dir is required"));
    }
    let options = DiscoverPackagesOptions {
        scan_dirs,
        disabled_plugins,
        runtime_version,
        use_cache,
    };
    let import = if include_import_args {
        Some(PluginManifestImportArgs {
            plugin: plugin
                .ok_or_else(|| "plugin-manifest import-symbol: --plugin is required".to_owned())?,
            symbol: symbol
                .ok_or_else(|| "plugin-manifest import-symbol: --symbol is required".to_owned())?,
            module_symbols,
        })
    } else {
        None
    };
    Ok((plan_json, options, import))
}

fn take_plugin_manifest_path(
    argv: &[String],
    index: usize,
    name: &str,
) -> Result<std::path::PathBuf, String> {
    Ok(std::path::PathBuf::from(take_plugin_manifest_value(
        argv, index, name,
    )?))
}

fn take_plugin_manifest_value(argv: &[String], index: usize, name: &str) -> Result<String, String> {
    argv.get(index + 1)
        .cloned()
        .ok_or_else(|| format!("plugin-manifest: missing {name} value"))
}

fn plugin_manifest_usage_error(message: &str) -> CliOutput {
    CliOutput {
        code: 2,
        stdout: String::new(),
        stderr: format!(
            "{message}\nusage: maw-rs plugin-manifest parse --dir <dir> --json <json> [--plan-json]\n       maw-rs plugin-manifest load --dir <dir> [--plan-json]\n       maw-rs plugin-manifest discover --scan-dir <dir>... [--disabled <name>]... [--runtime-version <version>] [--use-cache] [--plan-json]\n       maw-rs plugin-manifest import-symbol --scan-dir <dir>... --plugin <name> --symbol <name> [--module-symbol <name=value>]... [--disabled <name>]... [--runtime-version <version>] [--plan-json]\n       maw-rs plugin-manifest invoke --scan-dir <dir>... --plugin <name> [--source <cli|api|peer>] [--arg <arg>]... [--fake-ts-output <text>] [--fake-wasm-output <text>] [--disabled <name>]... [--runtime-version <version>] [--plan-json]\n"
        ),
    }
}

fn render_plugin_discover_json(
    options: &DiscoverPackagesOptions,
    plugins: &[LoadedPlugin],
    warnings: &[String],
) -> String {
    let scan_dirs = options
        .scan_dirs
        .iter()
        .map(path_string)
        .collect::<Vec<_>>();
    let plugin_json = plugins
        .iter()
        .map(render_loaded_plugin_json)
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"command\":\"plugin-manifest\",\"kind\":\"discover\",\"scanDirs\":{},\"runtimeVersion\":{},\"disabledPlugins\":{},\"useCache\":{},\"plugins\":[{plugin_json}],\"warnings\":{}}}\n",
        json_string_array(&scan_dirs),
        json_string(&options.runtime_version),
        json_string_array(&options.disabled_plugins),
        options.use_cache,
        json_string_array(warnings)
    )
}

fn render_plugin_import_symbol_json(
    plugin: &str,
    symbol: &str,
    value: &str,
    module_path: Option<&std::path::Path>,
    warnings: &[String],
) -> String {
    format!(
        "{{\"command\":\"plugin-manifest\",\"kind\":\"import-symbol\",\"plugin\":{},\"symbol\":{},\"value\":{},\"modulePath\":{},\"warnings\":{}}}\n",
        json_string(plugin),
        json_string(symbol),
        json_string(value),
        module_path.map_or_else(|| "null".to_owned(), |path| {
            json_string(&path_string(path))
        }),
        json_string_array(warnings)
    )
}

fn render_plugin_invoke_json(
    plugin: &str,
    ctx: &InvokeContext,
    result: &InvokeResult,
    runtime: &PlanInvokeRuntime,
    warnings: &[String],
) -> String {
    format!(
        "{{\"command\":\"plugin-manifest\",\"kind\":\"invoke\",\"plugin\":{},\"source\":{},\"args\":{},\"result\":{},\"runtime\":{{\"tsCalls\":{},\"wasmCalls\":{},\"lastWasmBytesLen\":{}}},\"warnings\":{}}}\n",
        json_string(plugin),
        json_string(ctx.source.as_str()),
        json_string_array(&ctx.args),
        render_invoke_result_json(result),
        runtime.ts_calls,
        runtime.wasm_calls,
        runtime.last_wasm_bytes_len,
        json_string_array(warnings)
    )
}

fn render_invoke_result_json(result: &InvokeResult) -> String {
    format!(
        "{{\"ok\":{},\"output\":{},\"error\":{}}}",
        result.ok,
        json_opt_string(result.output.as_deref()),
        json_opt_string(result.error.as_deref())
    )
}

fn render_loaded_plugin_json(plugin: &LoadedPlugin) -> String {
    format!(
        "{{\"dir\":{},\"wasmPath\":{},\"entryPath\":{},\"kind\":{},\"disabled\":{},\"manifest\":{}}}",
        json_string(&path_string(&plugin.dir)),
        json_string(&path_string(&plugin.wasm_path)),
        plugin.entry_path.as_ref().map_or_else(|| "null".to_owned(), |path| {
            json_string(&path_string(path))
        }),
        json_string(plugin.kind.as_str()),
        plugin.disabled,
        render_plugin_manifest_json(&plugin.manifest)
    )
}

fn render_plugin_manifest_json(manifest: &PluginManifest) -> String {
    let weight = manifest
        .weight
        .map_or_else(|| "null".to_owned(), |weight| weight.to_string());
    format!(
        "{{\"name\":{},\"version\":{},\"weight\":{weight},\"tier\":{},\"wasm\":{},\"entry\":{},\"sdk\":{},\"cli\":{},\"api\":{},\"description\":{},\"author\":{},\"target\":{},\"capabilityNamespaces\":{},\"capabilities\":{},\"capabilityWarnings\":{},\"artifact\":{}}}",
        json_string(&manifest.name),
        json_string(&manifest.version),
        manifest.tier.map_or_else(|| "null".to_owned(), |tier| json_string(tier.as_str())),
        json_opt_string(manifest.wasm.as_deref()),
        json_opt_string(manifest.entry.as_deref()),
        json_string(&manifest.sdk),
        render_plugin_cli_json(manifest.cli.as_ref()),
        render_plugin_api_json(manifest.api.as_ref()),
        json_opt_string(manifest.description.as_deref()),
        json_opt_string(manifest.author.as_deref()),
        manifest.target.map_or_else(|| "null".to_owned(), |target| json_string(target.as_str())),
        manifest.capability_namespaces.as_ref().map_or_else(|| "null".to_owned(), |values| json_string_array(values)),
        manifest.capabilities.as_ref().map_or_else(|| "null".to_owned(), |values| json_string_array(values)),
        json_string_array(&manifest.capability_warnings),
        manifest.artifact.as_ref().map_or_else(|| "null".to_owned(), |artifact| {
            format!(
                "{{\"path\":{},\"sha256\":{}}}",
                json_string(&artifact.path),
                json_opt_string(artifact.sha256.as_deref())
            )
        })
    )
}

fn render_plugin_cli_json(cli: Option<&maw_plugin_manifest::PluginCli>) -> String {
    let Some(cli) = cli else {
        return "null".to_owned();
    };
    let flags = cli.flags.as_ref().map_or_else(
        || "null".to_owned(),
        |flags| {
            let entries = flags
                .iter()
                .map(|(name, kind)| format!("{}:{}", json_string(name), json_string(kind.as_str())))
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{entries}}}")
        },
    );
    format!(
        "{{\"command\":{},\"aliases\":{},\"help\":{},\"flags\":{flags}}}",
        json_string(&cli.command),
        cli.aliases
            .as_ref()
            .map_or_else(|| "null".to_owned(), |values| json_string_array(values)),
        json_opt_string(cli.help.as_deref())
    )
}

fn render_plugin_api_json(api: Option<&maw_plugin_manifest::PluginApi>) -> String {
    let Some(api) = api else {
        return "null".to_owned();
    };
    let methods = api
        .methods
        .iter()
        .map(|method| method.as_str().to_owned())
        .collect::<Vec<_>>();
    format!(
        "{{\"path\":{},\"methods\":{}}}",
        json_string(&api.path),
        json_string_array(&methods)
    )
}

fn json_opt_string(value: Option<&str>) -> String {
    value.map_or_else(|| "null".to_owned(), json_string)
}

fn run_bind_host_plan(argv: &[String]) -> CliOutput {
    let parsed = match parse_bind_host_args(argv) {
        Ok(parsed) => parsed,
        Err(message) => return bind_host_usage_error(&message),
    };
    let result = resolve_bind_host(
        &parsed.config,
        parsed.maw_host.as_deref(),
        parsed.peers_store_len,
    );
    CliOutput {
        code: 0,
        stdout: if parsed.plan_json {
            render_bind_host_plan_json(&parsed.config, parsed.maw_host.as_deref(), &result)
        } else {
            format!("{}\n", result.hostname)
        },
        stderr: String::new(),
    }
}

struct BindHostArgs {
    plan_json: bool,
    config: BindConfig,
    maw_host: Option<String>,
    peers_store_len: Result<usize, String>,
}

fn parse_bind_host_args(argv: &[String]) -> Result<BindHostArgs, String> {
    let mut options = BindHostArgs {
        plan_json: false,
        config: BindConfig::default(),
        maw_host: None,
        peers_store_len: Ok(0),
    };

    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--plan-json" => options.plan_json = true,
            "--config-peers-len" => {
                let Some(value) = argv.get(index + 1) else {
                    return Err("bind-host: missing --config-peers-len value".to_owned());
                };
                options.config.peers_len = parse_usize_arg(value, "bind-host: --config-peers-len")?;
                index += 1;
            }
            "--config-named-peers-len" => {
                let Some(value) = argv.get(index + 1) else {
                    return Err("bind-host: missing --config-named-peers-len value".to_owned());
                };
                options.config.named_peers_len =
                    parse_usize_arg(value, "bind-host: --config-named-peers-len")?;
                index += 1;
            }
            "--maw-host" => {
                let Some(value) = argv.get(index + 1) else {
                    return Err("bind-host: missing --maw-host value".to_owned());
                };
                options.maw_host = Some(value.to_owned());
                index += 1;
            }
            "--peers-store-len" => {
                let Some(value) = argv.get(index + 1) else {
                    return Err("bind-host: missing --peers-store-len value".to_owned());
                };
                options.peers_store_len =
                    Ok(parse_usize_arg(value, "bind-host: --peers-store-len")?);
                index += 1;
            }
            "--peers-store-error" => {
                let Some(value) = argv.get(index + 1) else {
                    return Err("bind-host: missing --peers-store-error value".to_owned());
                };
                options.peers_store_len = Err(value.to_owned());
                index += 1;
            }
            arg => return Err(format!("bind-host: unknown argument {arg}")),
        }
        index += 1;
    }

    Ok(options)
}

fn render_bind_host_plan_json(
    config: &BindConfig,
    maw_host: Option<&str>,
    result: &BindHostResult,
) -> String {
    let mut input_fields = vec![
        format!("\"configPeersLen\":{}", config.peers_len),
        format!("\"configNamedPeersLen\":{}", config.named_peers_len),
    ];
    if let Some(maw_host) = maw_host {
        input_fields.push(format!("\"mawHost\":{}", json_string(maw_host)));
    }
    let reason = result
        .reason
        .map_or("null".to_owned(), |reason| json_string(reason.as_str()));
    format!(
        "{{\"command\":\"bind-host\",\"input\":{{{}}},\"hostname\":{},\"reason\":{reason}}}\n",
        input_fields.join(","),
        json_string(&result.hostname)
    )
}

fn bind_host_usage_error(message: &str) -> CliOutput {
    CliOutput {
        code: 2,
        stdout: String::new(),
        stderr: format!(
            "{message}\nusage: maw-rs bind-host [--config-peers-len <n>] [--config-named-peers-len <n>] [--maw-host <host>] [--peers-store-len <n>|--peers-store-error <err>] [--plan-json]\n"
        ),
    }
}

fn run_feed_plan(argv: &[String]) -> CliOutput {
    let (plan_json, action) = match parse_feed_plan_args(argv) {
        Ok(parsed) => parsed,
        Err(message) => return feed_usage_error(&message),
    };
    match action {
        FeedPlanAction::ParseLine { line } => render_feed_parse_plan(plan_json, &line),
        FeedPlanAction::Describe { event, message } => {
            let event = feed_event("oracle-a", 1_000_000, &event, &message);
            let description = describe_activity(&event);
            render_feed_description(plan_json, &event, &description)
        }
        FeedPlanAction::Active {
            now,
            window,
            events,
        } => render_feed_active(plan_json, now, window, &events),
    }
}

fn parse_feed_plan_args(argv: &[String]) -> Result<(bool, FeedPlanAction), String> {
    let mut parser = FeedArgParser {
        plan_json: false,
        action: None,
    };
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--plan-json" => parser.plan_json = true,
            "parse-line" | "--parse-line" => {
                let Some(value) = argv.get(index + 1) else {
                    return Err("feed: missing parse-line value".to_owned());
                };
                parser.action = Some(FeedPlanAction::ParseLine {
                    line: value.to_owned(),
                });
                index += 1;
            }
            "describe" | "--describe" => {
                let Some(value) = argv.get(index + 1) else {
                    return Err("feed: missing describe event value".to_owned());
                };
                parser.action = Some(FeedPlanAction::Describe {
                    event: value.to_owned(),
                    message: String::new(),
                });
                index += 1;
            }
            "active" | "--active" => {
                parser.action = Some(FeedPlanAction::Active {
                    now: 0,
                    window: 0,
                    events: Vec::new(),
                });
            }
            "--message" => {
                let Some(value) = argv.get(index + 1) else {
                    return Err("feed: missing --message value".to_owned());
                };
                parser.set_message(value)?;
                index += 1;
            }
            "--now" => {
                let Some(value) = argv.get(index + 1) else {
                    return Err("feed: missing --now value".to_owned());
                };
                parser.set_active_number(value, FeedNumberKind::Now)?;
                index += 1;
            }
            "--window" => {
                let Some(value) = argv.get(index + 1) else {
                    return Err("feed: missing --window value".to_owned());
                };
                parser.set_active_number(value, FeedNumberKind::Window)?;
                index += 1;
            }
            "--event" => {
                let Some(value) = argv.get(index + 1) else {
                    return Err("feed: missing --event value".to_owned());
                };
                parser.add_active_event(value)?;
                index += 1;
            }
            arg => return Err(format!("feed: unknown argument {arg}")),
        }
        index += 1;
    }
    parser.finish()
}

struct FeedArgParser {
    plan_json: bool,
    action: Option<FeedPlanAction>,
}

impl FeedArgParser {
    fn set_message(&mut self, value: &str) -> Result<(), String> {
        self.action = match self.action.take() {
            Some(FeedPlanAction::Describe { event, .. }) => Some(FeedPlanAction::Describe {
                event,
                message: value.to_owned(),
            }),
            _ => return Err("feed: --message requires describe".to_owned()),
        };
        Ok(())
    }

    fn set_active_number(&mut self, value: &str, kind: FeedNumberKind) -> Result<(), String> {
        let parsed = value
            .parse::<i64>()
            .map_err(|_| format!("feed: {} must be an integer", kind.name()))?;
        self.action = match self.action.take() {
            Some(FeedPlanAction::Active {
                mut now,
                mut window,
                events,
            }) => {
                match kind {
                    FeedNumberKind::Now => now = parsed,
                    FeedNumberKind::Window => window = parsed,
                }
                Some(FeedPlanAction::Active {
                    now,
                    window,
                    events,
                })
            }
            _ => return Err(format!("feed: {} requires active", kind.name())),
        };
        Ok(())
    }

    fn add_active_event(&mut self, value: &str) -> Result<(), String> {
        let event = parse_feed_event_spec(value)?;
        self.action = match self.action.take() {
            Some(FeedPlanAction::Active {
                now,
                window,
                mut events,
            }) => {
                events.push(event);
                Some(FeedPlanAction::Active {
                    now,
                    window,
                    events,
                })
            }
            _ => return Err("feed: --event requires active".to_owned()),
        };
        Ok(())
    }

    fn finish(self) -> Result<(bool, FeedPlanAction), String> {
        self.action.map_or_else(
            || Err("feed: expected parse-line, describe, or active".to_owned()),
            |action| Ok((self.plan_json, action)),
        )
    }
}

#[derive(Clone, Copy)]
enum FeedNumberKind {
    Now,
    Window,
}

impl FeedNumberKind {
    const fn name(self) -> &'static str {
        match self {
            Self::Now => "--now",
            Self::Window => "--window",
        }
    }
}

enum FeedPlanAction {
    ParseLine {
        line: String,
    },
    Describe {
        event: String,
        message: String,
    },
    Active {
        now: i64,
        window: i64,
        events: Vec<FeedEvent>,
    },
}

fn parse_feed_event_spec(value: &str) -> Result<FeedEvent, String> {
    let mut parts = value.splitn(3, ':');
    let oracle = parts.next().unwrap_or_default();
    let Some(ts) = parts.next() else {
        return Err("feed: --event must be oracle:ts:message".to_owned());
    };
    let message = parts.next().unwrap_or_default();
    let ts = ts
        .parse::<i64>()
        .map_err(|_| "feed: --event ts must be an integer".to_owned())?;
    Ok(feed_event(oracle, ts, "Notification", message))
}

fn feed_event(oracle: &str, ts: i64, event: &str, message: &str) -> FeedEvent {
    FeedEvent {
        timestamp: "2026-05-18 12:00:00".to_owned(),
        oracle: oracle.to_owned(),
        host: "m5".to_owned(),
        event: event.to_owned(),
        project: "maw-js".to_owned(),
        session_id: "s1".to_owned(),
        message: message.to_owned(),
        ts,
    }
}

fn render_feed_parse_plan(plan_json: bool, line: &str) -> CliOutput {
    let parsed = parse_line(line);
    CliOutput {
        code: i32::from(parsed.is_none()),
        stdout: if plan_json {
            match parsed {
                Some(event) => format!(
                    "{{\"command\":\"feed\",\"kind\":\"parseLine\",\"parsed\":true,\"event\":{}}}\n",
                    render_feed_event_json(&event)
                ),
                None => "{\"command\":\"feed\",\"kind\":\"parseLine\",\"parsed\":false}\n".to_owned(),
            }
        } else {
            parsed.map_or_else(String::new, |event| format!("{}\n", event.message))
        },
        stderr: String::new(),
    }
}

fn render_feed_description(plan_json: bool, event: &FeedEvent, description: &str) -> CliOutput {
    CliOutput {
        code: 0,
        stdout: if plan_json {
            format!(
                "{{\"command\":\"feed\",\"kind\":\"describe\",\"event\":{},\"description\":{}}}\n",
                render_feed_event_json(event),
                json_string(description)
            )
        } else {
            format!("{description}\n")
        },
        stderr: String::new(),
    }
}

fn render_feed_active(plan_json: bool, now: i64, window: i64, events: &[FeedEvent]) -> CliOutput {
    let active = active_oracles_at(events, now, window);
    let values: Vec<String> = active.values().map(render_feed_event_json).collect();
    CliOutput {
        code: 0,
        stdout: if plan_json {
            format!(
                "{{\"command\":\"feed\",\"kind\":\"active\",\"now\":{now},\"window\":{window},\"active\":[{}]}}\n",
                values.join(",")
            )
        } else {
            format!(
                "{}\n",
                active.keys().cloned().collect::<Vec<_>>().join("\n")
            )
        },
        stderr: String::new(),
    }
}

fn render_feed_event_json(event: &FeedEvent) -> String {
    format!(
        "{{\"timestamp\":{},\"oracle\":{},\"host\":{},\"event\":{},\"project\":{},\"sessionId\":{},\"message\":{},\"ts\":{}}}",
        json_string(&event.timestamp),
        json_string(&event.oracle),
        json_string(&event.host),
        json_string(&event.event),
        json_string(&event.project),
        json_string(&event.session_id),
        json_string(&event.message),
        event.ts
    )
}

fn feed_usage_error(message: &str) -> CliOutput {
    CliOutput {
        code: 2,
        stdout: String::new(),
        stderr: format!(
            "{message}\nusage: maw-rs feed parse-line <line> [--plan-json]\n       maw-rs feed describe <event> [--message <message>] [--plan-json]\n       maw-rs feed active --now <ms> --window <ms> [--event <oracle:ts:message>]... [--plan-json]\n"
        ),
    }
}

fn run_fuzzy_plan(argv: &[String]) -> CliOutput {
    let (plan_json, action) = match parse_fuzzy_plan_args(argv) {
        Ok(parsed) => parsed,
        Err(message) => return fuzzy_usage_error(&message),
    };

    match action {
        FuzzyPlanAction::Distance { left, right } => {
            render_fuzzy_distance(plan_json, &left, &right)
        }
        FuzzyPlanAction::Match {
            input,
            candidates,
            max_results,
            max_distance,
        } => render_fuzzy_match(plan_json, &input, &candidates, max_results, max_distance),
    }
}

fn parse_fuzzy_plan_args(argv: &[String]) -> Result<(bool, FuzzyPlanAction), String> {
    let mut plan_json = false;
    let mut action = None;
    let mut max_results = 3;
    let mut max_distance = 3;
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--plan-json" => plan_json = true,
            "distance" | "--distance" => {
                let Some(left) = argv.get(index + 1) else {
                    return Err("fuzzy: missing distance left value".to_owned());
                };
                let Some(right) = argv.get(index + 2) else {
                    return Err("fuzzy: missing distance right value".to_owned());
                };
                action = Some(FuzzyPlanAction::Distance {
                    left: left.to_owned(),
                    right: right.to_owned(),
                });
                index += 2;
            }
            "match" | "--match" => {
                let Some(input) = argv.get(index + 1) else {
                    return Err("fuzzy: missing match input".to_owned());
                };
                action = Some(FuzzyPlanAction::Match {
                    input: input.to_owned(),
                    candidates: Vec::new(),
                    max_results,
                    max_distance,
                });
                index += 1;
            }
            "--candidate" => {
                let Some(value) = argv.get(index + 1) else {
                    return Err("fuzzy: missing --candidate value".to_owned());
                };
                action = append_fuzzy_candidate(action, value)?;
                index += 1;
            }
            "--max-results" => {
                let Some(value) = argv.get(index + 1) else {
                    return Err("fuzzy: missing --max-results value".to_owned());
                };
                max_results = parse_usize_arg(value, "fuzzy: --max-results")?;
                action = update_fuzzy_limits(action, max_results, max_distance);
                index += 1;
            }
            "--max-distance" => {
                let Some(value) = argv.get(index + 1) else {
                    return Err("fuzzy: missing --max-distance value".to_owned());
                };
                max_distance = parse_usize_arg(value, "fuzzy: --max-distance")?;
                action = update_fuzzy_limits(action, max_results, max_distance);
                index += 1;
            }
            arg => return Err(format!("fuzzy: unknown argument {arg}")),
        }
        index += 1;
    }

    action.map_or_else(
        || Err("fuzzy: expected distance or match".to_owned()),
        |action| Ok((plan_json, action)),
    )
}

fn append_fuzzy_candidate(
    action: Option<FuzzyPlanAction>,
    value: &str,
) -> Result<Option<FuzzyPlanAction>, String> {
    match action {
        Some(FuzzyPlanAction::Match {
            input,
            mut candidates,
            max_results,
            max_distance,
        }) => {
            candidates.push(value.to_owned());
            Ok(Some(FuzzyPlanAction::Match {
                input,
                candidates,
                max_results,
                max_distance,
            }))
        }
        _ => Err("fuzzy: --candidate requires match".to_owned()),
    }
}

fn update_fuzzy_limits(
    action: Option<FuzzyPlanAction>,
    max_results: usize,
    max_distance: usize,
) -> Option<FuzzyPlanAction> {
    match action {
        Some(FuzzyPlanAction::Match {
            input, candidates, ..
        }) => Some(FuzzyPlanAction::Match {
            input,
            candidates,
            max_results,
            max_distance,
        }),
        action => action,
    }
}

fn parse_usize_arg(value: &str, name: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .map_err(|_| format!("{name} must be a non-negative integer"))
}

fn render_fuzzy_distance(plan_json: bool, left: &str, right: &str) -> CliOutput {
    let distance = fuzzy_distance(left, right);
    CliOutput {
        code: 0,
        stdout: if plan_json {
            format!(
                "{{\"command\":\"fuzzy\",\"kind\":\"distance\",\"left\":{},\"right\":{},\"distance\":{distance}}}\n",
                json_string(left),
                json_string(right)
            )
        } else {
            format!("{distance}\n")
        },
        stderr: String::new(),
    }
}

fn render_fuzzy_match(
    plan_json: bool,
    input: &str,
    candidates: &[String],
    max_results: usize,
    max_distance: usize,
) -> CliOutput {
    let candidate_refs: Vec<&str> = candidates.iter().map(String::as_str).collect();
    let matches = fuzzy_match(input, &candidate_refs, max_results, max_distance);
    CliOutput {
        code: 0,
        stdout: if plan_json {
            format!(
                "{{\"command\":\"fuzzy\",\"kind\":\"match\",\"input\":{},\"candidates\":{},\"maxResults\":{max_results},\"maxDistance\":{max_distance},\"matches\":{}}}\n",
                json_string(input),
                json_string_array(candidates),
                json_string_array(&matches)
            )
        } else {
            format!("{}\n", matches.join("\n"))
        },
        stderr: String::new(),
    }
}

enum FuzzyPlanAction {
    Distance {
        left: String,
        right: String,
    },
    Match {
        input: String,
        candidates: Vec<String>,
        max_results: usize,
        max_distance: usize,
    },
}

fn fuzzy_usage_error(message: &str) -> CliOutput {
    CliOutput {
        code: 2,
        stdout: String::new(),
        stderr: format!(
            "{message}\nusage: maw-rs fuzzy distance <left> <right> [--plan-json]\n       maw-rs fuzzy match <input> [--candidate <candidate>]... [--max-results <n>] [--max-distance <n>] [--plan-json]\n"
        ),
    }
}

fn run_identity_plan(argv: &[String]) -> CliOutput {
    let (plan_json, action) = match parse_identity_plan_args(argv) {
        Ok(parsed) => parsed,
        Err(message) => return identity_usage_error(&message),
    };
    match action {
        IdentityPlanAction::SessionName { oracle, slot } => {
            let input = CanonicalSessionNameInput { oracle, slot };
            match canonical_session_name(&input) {
                Ok(canonical) => CliOutput {
                    code: 0,
                    stdout: if plan_json {
                        render_identity_session_plan_json(&input, &canonical)
                    } else {
                        format!("{canonical}\n")
                    },
                    stderr: String::new(),
                },
                Err(error) => CliOutput {
                    code: 1,
                    stdout: String::new(),
                    stderr: format!("identity: {error}\n"),
                },
            }
        }
        IdentityPlanAction::Node { host, user } => {
            let canonical = canonical_node_identity(&host, user.as_deref());
            CliOutput {
                code: 0,
                stdout: if plan_json {
                    render_identity_node_plan_json(&host, user.as_deref(), &canonical)
                } else {
                    format!("{canonical}\n")
                },
                stderr: String::new(),
            }
        }
    }
}

fn parse_identity_plan_args(argv: &[String]) -> Result<(bool, IdentityPlanAction), String> {
    let mut plan_json = false;
    let mut action = None;
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--plan-json" => plan_json = true,
            "session-name" | "--session-name" => {
                let Some(oracle) = argv.get(index + 1) else {
                    return Err("identity: missing session-name oracle".to_owned());
                };
                action = Some(IdentityPlanAction::SessionName {
                    oracle: oracle.to_owned(),
                    slot: None,
                });
                index += 1;
            }
            "node" | "node-identity" | "--node-identity" => {
                let Some(host) = argv.get(index + 1) else {
                    return Err("identity: missing node host".to_owned());
                };
                action = Some(IdentityPlanAction::Node {
                    host: host.to_owned(),
                    user: None,
                });
                index += 1;
            }
            "--slot" => {
                let Some(value) = argv.get(index + 1) else {
                    return Err("identity: missing --slot value".to_owned());
                };
                let Ok(slot) = value.parse::<u32>() else {
                    return Err("identity: --slot must be an integer".to_owned());
                };
                action = match action {
                    Some(IdentityPlanAction::SessionName { oracle, .. }) => {
                        Some(IdentityPlanAction::SessionName {
                            oracle,
                            slot: Some(slot),
                        })
                    }
                    _ => return Err("identity: --slot requires session-name".to_owned()),
                };
                index += 1;
            }
            "--user" => {
                let Some(value) = argv.get(index + 1) else {
                    return Err("identity: missing --user value".to_owned());
                };
                action = match action {
                    Some(IdentityPlanAction::Node { host, .. }) => Some(IdentityPlanAction::Node {
                        host,
                        user: Some(value.to_owned()),
                    }),
                    _ => return Err("identity: --user requires node-identity".to_owned()),
                };
                index += 1;
            }
            arg => return Err(format!("identity: unknown argument {arg}")),
        }
        index += 1;
    }
    action.map_or_else(
        || Err("identity: expected session-name or node-identity".to_owned()),
        |action| Ok((plan_json, action)),
    )
}

enum IdentityPlanAction {
    SessionName { oracle: String, slot: Option<u32> },
    Node { host: String, user: Option<String> },
}

fn identity_usage_error(message: &str) -> CliOutput {
    CliOutput {
        code: 2,
        stdout: String::new(),
        stderr: format!(
            "{message}\nusage: maw-rs identity session-name <oracle> [--slot <0-99>] [--plan-json]\n       maw-rs identity node-identity <host> [--user <user>] [--plan-json]\n"
        ),
    }
}

fn render_identity_session_plan_json(input: &CanonicalSessionNameInput, canonical: &str) -> String {
    let mut input_fields = vec![format!("\"oracle\":{}", json_string(&input.oracle))];
    if let Some(slot) = input.slot {
        input_fields.push(format!("\"slot\":{slot}"));
    }
    format!(
        "{{\"command\":\"identity\",\"kind\":\"sessionName\",\"input\":{{{}}},\"canonical\":{}}}\n",
        input_fields.join(","),
        json_string(canonical)
    )
}

fn render_identity_node_plan_json(host: &str, user: Option<&str>, canonical: &str) -> String {
    let mut input_fields = vec![format!("\"host\":{}", json_string(host))];
    if let Some(user) = user {
        input_fields.push(format!("\"user\":{}", json_string(user)));
    }
    format!(
        "{{\"command\":\"identity\",\"kind\":\"nodeIdentity\",\"input\":{{{}}},\"canonical\":{}}}\n",
        input_fields.join(","),
        json_string(canonical)
    )
}

fn run_policy_plan(argv: &[String]) -> CliOutput {
    let (plan_json, action) = match parse_policy_plan_args(argv) {
        Ok(parsed) => parsed,
        Err(message) => return policy_usage_error(&message),
    };
    render_policy_plan(action, plan_json)
}

fn parse_policy_plan_args(argv: &[String]) -> Result<(bool, PolicyPlanAction), String> {
    let mut plan_json = false;
    let mut action = PolicyPlanAction::Constants;

    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--plan-json" => plan_json = true,
            "--constants" => action = PolicyPlanAction::Constants,
            "--weight" => {
                let Some(value) = argv.get(index + 1) else {
                    return Err("policy: missing --weight value".to_owned());
                };
                let Ok(weight) = value.parse::<i32>() else {
                    return Err("policy: --weight must be an integer".to_owned());
                };
                action = PolicyPlanAction::WeightToTier(weight);
                index += 1;
            }
            "--default-active" => {
                let Some(value) = argv.get(index + 1) else {
                    return Err("policy: missing --default-active value".to_owned());
                };
                action = PolicyPlanAction::DefaultActiveGroup(value.to_owned());
                index += 1;
            }
            "--includes" => {
                let Some(value) = argv.get(index + 1) else {
                    return Err("policy: missing --includes value".to_owned());
                };
                action = match action {
                    PolicyPlanAction::DefaultActiveGroup(key) => {
                        PolicyPlanAction::DefaultActiveIncludes {
                            key,
                            plugin: value.to_owned(),
                        }
                    }
                    _ => {
                        return Err("policy: --includes requires --default-active <key>".to_owned())
                    }
                };
                index += 1;
            }
            arg => return Err(format!("policy: unknown argument {arg}")),
        }
        index += 1;
    }
    Ok((plan_json, action))
}

fn render_policy_plan(action: PolicyPlanAction, plan_json: bool) -> CliOutput {
    match action {
        PolicyPlanAction::Constants => CliOutput {
            code: 0,
            stdout: if plan_json {
                render_policy_constants_json()
            } else {
                format!(
                    "policy constants default-tier={} known-tiers={}\n",
                    DEFAULT_TIER.as_str(),
                    KNOWN_TIERS
                        .iter()
                        .map(|tier| tier.as_str())
                        .collect::<Vec<_>>()
                        .join(",")
                )
            },
            stderr: String::new(),
        },
        PolicyPlanAction::WeightToTier(weight) => {
            let tier = weight_to_tier(weight);
            CliOutput {
                code: 0,
                stdout: if plan_json {
                    format!(
                        "{{\"command\":\"policy\",\"kind\":\"weightToTier\",\"weight\":{weight},\"tier\":{}}}\n",
                        json_string(tier.as_str())
                    )
                } else {
                    format!("policy weight {weight}: {}\n", tier.as_str())
                },
                stderr: String::new(),
            }
        }
        PolicyPlanAction::DefaultActiveGroup(key) => {
            let Some(group) = default_active_group(&key) else {
                return policy_usage_error("policy: unknown --default-active key");
            };
            CliOutput {
                code: 0,
                stdout: if plan_json {
                    render_policy_default_active_json(&key, group)
                } else {
                    format!(
                        "policy default-active {key}: migration={} plugins={}\n",
                        group.migration,
                        group.plugins.join(",")
                    )
                },
                stderr: String::new(),
            }
        }
        PolicyPlanAction::DefaultActiveIncludes { key, plugin } => {
            let Some(group) = default_active_group(&key) else {
                return policy_usage_error("policy: unknown --default-active key");
            };
            let included = (group.includes)(&plugin);
            CliOutput {
                code: 0,
                stdout: if plan_json {
                    format!(
                        "{{\"command\":\"policy\",\"kind\":\"defaultActiveIncludes\",\"key\":{},\"plugin\":{},\"included\":{included}}}\n",
                        json_string(&key),
                        json_string(&plugin)
                    )
                } else {
                    format!("policy default-active {key} includes {plugin}: {included}\n")
                },
                stderr: String::new(),
            }
        }
    }
}

enum PolicyPlanAction {
    Constants,
    WeightToTier(i32),
    DefaultActiveGroup(String),
    DefaultActiveIncludes { key: String, plugin: String },
}

fn policy_usage_error(message: &str) -> CliOutput {
    CliOutput {
        code: 2,
        stdout: String::new(),
        stderr: format!(
            "{message}\nusage: maw-rs policy [--constants|--weight <i32>|--default-active <key> [--includes <plugin>]] [--plan-json]\n"
        ),
    }
}

fn render_policy_constants_json() -> String {
    let tiers: Vec<&str> = KNOWN_TIERS.iter().map(|tier| tier.as_str()).collect();
    format!(
        "{{\"command\":\"policy\",\"kind\":\"constants\",\"knownTiers\":{},\"defaultTier\":{}}}\n",
        json_str_array(&tiers),
        json_string(DEFAULT_TIER.as_str())
    )
}

fn render_policy_default_active_json(key: &str, group: maw_policy::DefaultActiveGroup) -> String {
    format!(
        "{{\"command\":\"policy\",\"kind\":\"defaultActiveGroup\",\"key\":{},\"migration\":{},\"plugins\":{}}}\n",
        json_string(key),
        json_string(group.migration),
        json_str_array(group.plugins)
    )
}

fn run_transport_plan(argv: &[String]) -> CliOutput {
    let mut plan_json = false;
    let mut classify = None;
    let mut should_send = false;
    let mut transport_specs = Vec::new();

    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--plan-json" => plan_json = true,
            "--classify-error" => {
                let Some(value) = argv.get(index + 1) else {
                    return transport_usage_error("transport: missing --classify-error value");
                };
                classify = Some(value.to_owned());
                index += 1;
            }
            "--classify-empty" => classify = Some(String::new()),
            "--send" => should_send = true,
            "--transport" => {
                let Some(value) = argv.get(index + 1) else {
                    return transport_usage_error("transport: missing --transport value");
                };
                match parse_transport_spec(value) {
                    Ok(transport) => transport_specs.push(transport),
                    Err(message) => return transport_usage_error(&message),
                }
                index += 1;
            }
            arg => return transport_usage_error(&format!("transport: unknown argument {arg}")),
        }
        index += 1;
    }

    if let Some(error) = classify {
        let classified = if error.is_empty() {
            classify_error(None)
        } else {
            classify_error(Some(&error))
        };
        return CliOutput {
            code: 0,
            stdout: if plan_json {
                format!(
                    "{{\"command\":\"transport\",\"kind\":\"classifyError\",\"reason\":{},\"retryable\":{}}}\n",
                    json_string(classified.reason.as_str()),
                    classified.retryable
                )
            } else {
                format!(
                    "transport classify reason={} retryable={}\n",
                    classified.reason.as_str(),
                    classified.retryable
                )
            },
            stderr: String::new(),
        };
    }

    if !should_send {
        return transport_usage_error("transport: expected --classify-error or --send");
    }

    let sent_order = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut router = TransportRouter::new();
    for spec in transport_specs {
        router.register(CliTransport {
            spec,
            sent: std::rc::Rc::clone(&sent_order),
        });
    }
    let target = TransportTarget {
        oracle: "neo".to_owned(),
        host: None,
        tmux_target: Some("neo:1".to_owned()),
    };
    let result = router.send(&target, "hello", "codex");
    CliOutput {
        code: 0,
        stdout: if plan_json {
            render_transport_send_plan_json(&result, &sent_order.borrow())
        } else {
            render_transport_send_plan_text(&result, &sent_order.borrow())
        },
        stderr: String::new(),
    }
}

fn transport_usage_error(message: &str) -> CliOutput {
    CliOutput {
        code: 2,
        stdout: String::new(),
        stderr: format!(
            "{message}\nusage: maw-rs transport --classify-error <error>|--classify-empty|--send [--transport <name[:connected][:canReach][:ok|false|throw=err]>]... [--plan-json]\n"
        ),
    }
}

#[derive(Debug, Clone)]
struct CliTransportSpec {
    name: String,
    connected: bool,
    can_reach: bool,
    action: CliTransportAction,
}

#[derive(Debug, Clone)]
enum CliTransportAction {
    Ok,
    False,
    Throw(String),
}

fn parse_transport_spec(value: &str) -> Result<CliTransportSpec, String> {
    let mut parts = value.splitn(4, ':');
    let name = parts.next().unwrap_or_default();
    if name.is_empty() {
        return Err("transport: --transport requires a name".to_owned());
    }
    let connected = parse_optional_bool(parts.next(), true, "connected")?;
    let can_reach = parse_optional_bool(parts.next(), true, "canReach")?;
    let action = match parts.next() {
        None | Some("" | "ok") => CliTransportAction::Ok,
        Some("false") => CliTransportAction::False,
        Some(value) => {
            let Some(error) = value.strip_prefix("throw=") else {
                return Err("transport: action must be ok, false, or throw=<error>".to_owned());
            };
            CliTransportAction::Throw(error.to_owned())
        }
    };
    Ok(CliTransportSpec {
        name: name.to_owned(),
        connected,
        can_reach,
        action,
    })
}

fn parse_optional_bool(value: Option<&str>, default: bool, name: &str) -> Result<bool, String> {
    match value {
        None | Some("") => Ok(default),
        Some("true") => Ok(true),
        Some("false") => Ok(false),
        Some(_) => Err(format!("transport: invalid {name} boolean")),
    }
}

struct CliTransport {
    spec: CliTransportSpec,
    sent: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
}

impl Transport for CliTransport {
    fn name(&self) -> &str {
        &self.spec.name
    }

    fn connected(&self) -> bool {
        self.spec.connected
    }

    fn can_reach(&self, _target: &TransportTarget) -> bool {
        self.spec.can_reach
    }

    fn send(
        &mut self,
        _target: &TransportTarget,
        _message: &str,
        _from: &str,
    ) -> Result<bool, String> {
        self.sent.borrow_mut().push(self.spec.name.clone());
        match &self.spec.action {
            CliTransportAction::Ok => Ok(true),
            CliTransportAction::False => Ok(false),
            CliTransportAction::Throw(error) => Err(error.clone()),
        }
    }
}

fn render_transport_send_plan_json(result: &TransportResult, sent: &[String]) -> String {
    let mut fields = vec![
        "\"command\":\"transport\"".to_owned(),
        "\"kind\":\"send\"".to_owned(),
        format!("\"ok\":{}", result.ok),
        format!("\"via\":{}", json_string(&result.via)),
        format!("\"retryable\":{}", result.retryable),
        format!("\"sent\":{}", json_string_array(sent)),
    ];
    if let Some(reason) = result.reason {
        fields.push(format!("\"reason\":{}", json_string(reason.as_str())));
    }
    format!("{{{}}}\n", fields.join(","))
}

fn render_transport_send_plan_text(result: &TransportResult, sent: &[String]) -> String {
    let reason = result.reason.map_or("-", TransportFailureReason::as_str);
    format!(
        "transport send ok={} via={} reason={} retryable={} sent={}\n",
        result.ok,
        result.via,
        reason,
        result.retryable,
        sent.join(",")
    )
}

fn run_split_policy_plan(argv: &[String]) -> CliOutput {
    let mut plan_json = false;
    let mut pane_current_command = None;
    let mut requested_policy = None;
    let mut no_attach = false;
    let mut force_split = false;

    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--plan-json" => plan_json = true,
            "--pane-current-command" => {
                let Some(value) = argv.get(index + 1) else {
                    return split_policy_usage_error(
                        "split-policy: missing --pane-current-command value",
                    );
                };
                pane_current_command = Some(value.to_owned());
                index += 1;
            }
            "--requested-policy" | "--claude-pane-policy" => {
                let Some(value) = argv.get(index + 1) else {
                    return split_policy_usage_error(
                        "split-policy: missing --requested-policy value",
                    );
                };
                requested_policy = Some(value.to_owned());
                index += 1;
            }
            "--no-attach" => no_attach = true,
            "--force-split" => force_split = true,
            arg => {
                return split_policy_usage_error(&format!("split-policy: unknown argument {arg}"))
            }
        }
        index += 1;
    }

    let input = SplitPolicyInput {
        pane_current_command,
        no_attach,
        requested_policy,
        force_split,
    };

    match decide_split_policy(&input) {
        Ok(decision) => CliOutput {
            code: 0,
            stdout: if plan_json {
                render_split_policy_plan_json(decision)
            } else {
                render_split_policy_plan_text(decision)
            },
            stderr: String::new(),
        },
        Err(error) => CliOutput {
            code: 2,
            stdout: String::new(),
            stderr: format!("split-policy: {error}\n"),
        },
    }
}

fn split_policy_usage_error(message: &str) -> CliOutput {
    CliOutput {
        code: 2,
        stdout: String::new(),
        stderr: format!(
            "{message}\nusage: maw-rs split-policy [--pane-current-command <cmd>] [--requested-policy <policy>] [--no-attach] [--force-split] [--plan-json]\n"
        ),
    }
}

fn render_split_policy_plan_json(decision: SplitPolicyDecision) -> String {
    format!(
        "{{\"command\":\"split-policy\",\"action\":{},\"reason\":{}}}\n",
        json_string(decision.action.as_str()),
        json_string(decision.reason.as_str())
    )
}

fn render_split_policy_plan_text(decision: SplitPolicyDecision) -> String {
    format!(
        "split-policy action={} reason={}\n",
        decision.action.as_str(),
        decision.reason.as_str()
    )
}

fn run_peer_probe_plan(argv: &[String]) -> CliOutput {
    let Some(action) = argv.first().map(String::as_str) else {
        return peer_probe_usage_error("peer-probe: missing action");
    };
    match action {
        "classify" => run_peer_probe_classify_plan(&argv[1..]),
        "format" => run_peer_probe_format_plan(&argv[1..]),
        "handshake" => run_peer_probe_handshake_plan(&argv[1..]),
        _ => peer_probe_usage_error("peer-probe: invalid action"),
    }
}

fn run_peer_probe_classify_plan(argv: &[String]) -> CliOutput {
    let mut plan_json = false;
    let mut input = None;
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--plan-json" => plan_json = true,
            "--http-status" => {
                let Some(value) = argv.get(index + 1) else {
                    return peer_probe_usage_error(
                        "peer-probe classify: missing --http-status value",
                    );
                };
                let Ok(status) = value.parse::<u16>() else {
                    return peer_probe_usage_error(
                        "peer-probe classify: --http-status must be an integer",
                    );
                };
                input = Some(ProbeFailureInput::Http { status, ok: false });
                index += 1;
            }
            "--code" => {
                let Some(value) = argv.get(index + 1) else {
                    return peer_probe_usage_error("peer-probe classify: missing --code value");
                };
                input = Some(ProbeFailureInput::Code(value.to_owned()));
                index += 1;
            }
            "--cause-code" => {
                let Some(value) = argv.get(index + 1) else {
                    return peer_probe_usage_error(
                        "peer-probe classify: missing --cause-code value",
                    );
                };
                input = Some(ProbeFailureInput::CauseCode(value.to_owned()));
                index += 1;
            }
            "--name" => {
                let Some(value) = argv.get(index + 1) else {
                    return peer_probe_usage_error("peer-probe classify: missing --name value");
                };
                input = Some(ProbeFailureInput::Name(value.to_owned()));
                index += 1;
            }
            "--non-object" => input = Some(ProbeFailureInput::NonObject),
            arg => {
                return peer_probe_usage_error(&format!(
                    "peer-probe classify: unknown argument {arg}"
                ))
            }
        }
        index += 1;
    }
    let Some(input) = input else {
        return peer_probe_usage_error("peer-probe classify: missing input");
    };
    let code = classify_probe_error(&input);
    CliOutput {
        code: 0,
        stdout: if plan_json {
            format!(
                "{{\"command\":\"peer-probe\",\"action\":\"classify\",\"ok\":true,\"code\":{},\"exitCode\":{},\"hint\":{}}}\n",
                json_string(code.as_str()),
                probe_exit_code(code),
                json_string(maw_peer::probe_hint(code))
            )
        } else {
            format!("{}\n", code.as_str())
        },
        stderr: String::new(),
    }
}

fn run_peer_probe_format_plan(argv: &[String]) -> CliOutput {
    let mut plan_json = false;
    let mut code = None;
    let mut message = None;
    let mut at = "now".to_owned();
    let mut url = None;
    let mut alias = None;
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--plan-json" => plan_json = true,
            "--code" => {
                let Some(value) = argv.get(index + 1) else {
                    return peer_probe_usage_error("peer-probe format: missing --code value");
                };
                code = parse_probe_error_code(value);
                if code.is_none() {
                    return peer_probe_usage_error("peer-probe format: invalid --code value");
                }
                index += 1;
            }
            "--message" => {
                let Some(value) = argv.get(index + 1) else {
                    return peer_probe_usage_error("peer-probe format: missing --message value");
                };
                message = Some(value.to_owned());
                index += 1;
            }
            "--at" => {
                let Some(value) = argv.get(index + 1) else {
                    return peer_probe_usage_error("peer-probe format: missing --at value");
                };
                value.clone_into(&mut at);
                index += 1;
            }
            "--url" => {
                let Some(value) = argv.get(index + 1) else {
                    return peer_probe_usage_error("peer-probe format: missing --url value");
                };
                url = Some(value.to_owned());
                index += 1;
            }
            "--alias" => {
                let Some(value) = argv.get(index + 1) else {
                    return peer_probe_usage_error("peer-probe format: missing --alias value");
                };
                alias = Some(value.to_owned());
                index += 1;
            }
            arg => {
                return peer_probe_usage_error(&format!(
                    "peer-probe format: unknown argument {arg}"
                ))
            }
        }
        index += 1;
    }
    let (Some(code), Some(message), Some(url), Some(alias)) = (code, message, url, alias) else {
        return peer_probe_usage_error("peer-probe format: missing required value");
    };
    let err = ProbeLastError { code, message, at };
    let formatted = format_probe_error(&err, &url, &alias);
    CliOutput {
        code: 0,
        stdout: if plan_json {
            format!(
                "{{\"command\":\"peer-probe\",\"action\":\"format\",\"ok\":true,\"code\":{},\"host\":{},\"hint\":{},\"formatted\":{}}}\n",
                json_string(code.as_str()),
                json_string(&safe_probe_host(&url)),
                json_string(pick_probe_hint(&err)),
                json_string(&formatted)
            )
        } else {
            formatted + "\n"
        },
        stderr: String::new(),
    }
}

fn run_peer_probe_handshake_plan(argv: &[String]) -> CliOutput {
    let mut plan_json = false;
    let mut handshake = None;
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--plan-json" => plan_json = true,
            "--legacy-true" => handshake = Some(ProbeMawHandshake::LegacyTrue),
            "--schema" => {
                let Some(value) = argv.get(index + 1) else {
                    return peer_probe_usage_error("peer-probe handshake: missing --schema value");
                };
                handshake = Some(ProbeMawHandshake::SchemaObject(value.to_owned()));
                index += 1;
            }
            "--empty-object" => handshake = Some(ProbeMawHandshake::EmptyObject),
            "--other-truthy" => handshake = Some(ProbeMawHandshake::OtherTruthy),
            "--missing" => handshake = Some(ProbeMawHandshake::Missing),
            arg => {
                return peer_probe_usage_error(&format!(
                    "peer-probe handshake: unknown argument {arg}"
                ))
            }
        }
        index += 1;
    }
    let Some(handshake) = handshake else {
        return peer_probe_usage_error("peer-probe handshake: missing shape");
    };
    let valid = is_valid_maw_handshake(&handshake);
    CliOutput {
        code: 0,
        stdout: if plan_json {
            format!(
                "{{\"command\":\"peer-probe\",\"action\":\"handshake\",\"ok\":true,\"valid\":{valid}}}\n"
            )
        } else {
            format!("valid={valid}\n")
        },
        stderr: String::new(),
    }
}

fn parse_probe_error_code(value: &str) -> Option<ProbeErrorCode> {
    match value {
        "DNS" => Some(ProbeErrorCode::Dns),
        "REFUSED" => Some(ProbeErrorCode::Refused),
        "TIMEOUT" => Some(ProbeErrorCode::Timeout),
        "HTTP_4XX" => Some(ProbeErrorCode::Http4xx),
        "HTTP_5XX" => Some(ProbeErrorCode::Http5xx),
        "TLS" => Some(ProbeErrorCode::Tls),
        "BAD_BODY" => Some(ProbeErrorCode::BadBody),
        "UNKNOWN" => Some(ProbeErrorCode::Unknown),
        _ => None,
    }
}

fn peer_probe_usage_error(message: &str) -> CliOutput {
    CliOutput {
        code: 2,
        stdout: String::new(),
        stderr: format!("{message}\n{}\n", peer_probe_usage()),
    }
}

fn peer_probe_usage() -> &'static str {
    "usage: maw-rs peer-probe classify (--http-status <n>|--code <code>|--cause-code <code>|--name <name>|--non-object) [--plan-json]\n       maw-rs peer-probe format --code <code> --message <msg> --url <url> --alias <alias> [--at <ts>] [--plan-json]\n       maw-rs peer-probe handshake (--legacy-true|--schema <schema>|--empty-object|--other-truthy|--missing) [--plan-json]"
}

fn run_peer_sources_plan(argv: &[String]) -> CliOutput {
    let mut plan_json = false;
    let mut mode = PeerSourceMode::Both;
    let mut config = PeerConfig::default();
    let mut discoveries: Option<DiscoveryResult> = None;
    let mut discovery_rows = Vec::new();
    let mut discovery_error_hint = None;

    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--plan-json" => plan_json = true,
            "--mode" => {
                let Some(value) = argv.get(index + 1) else {
                    return peer_sources_usage_error("peer-sources: missing --mode value");
                };
                let Some(parsed) = maw_peer::parse_peer_source_mode(Some(value), mode) else {
                    return peer_sources_usage_error("peer-sources: unknown --mode");
                };
                mode = parsed;
                index += 1;
            }
            "--peer" => {
                let Some(value) = argv.get(index + 1) else {
                    return peer_sources_usage_error("peer-sources: missing --peer value");
                };
                config.peers.push(value.to_owned());
                index += 1;
            }
            "--named-peer" => {
                let Some(value) = argv.get(index + 1) else {
                    return peer_sources_usage_error("peer-sources: missing --named-peer value");
                };
                match parse_key_value(value, "peer-sources: --named-peer must use <name=url>") {
                    Ok((name, url)) => config.named_peers.push(NamedPeerConfig { name, url }),
                    Err(message) => return peer_sources_usage_error(&message),
                }
                index += 1;
            }
            "--discovery-ok" => discoveries = Some(DiscoveryResult::Ok { peers: Vec::new() }),
            "--discovery-error" => {
                let Some(value) = argv.get(index + 1) else {
                    return peer_sources_usage_error(
                        "peer-sources: missing --discovery-error value",
                    );
                };
                discoveries = Some(DiscoveryResult::Err {
                    error: value.to_owned(),
                    hint: discovery_error_hint.clone(),
                });
                index += 1;
            }
            "--discovery-hint" => {
                let Some(value) = argv.get(index + 1) else {
                    return peer_sources_usage_error(
                        "peer-sources: missing --discovery-hint value",
                    );
                };
                discovery_error_hint = Some(value.to_owned());
                if let Some(DiscoveryResult::Err { hint, .. }) = &mut discoveries {
                    hint.clone_from(&discovery_error_hint);
                }
                index += 1;
            }
            "--discovered" => {
                let Some(value) = argv.get(index + 1) else {
                    return peer_sources_usage_error("peer-sources: missing --discovered value");
                };
                match parse_discovery_row(value) {
                    Ok(row) => discovery_rows.push(row),
                    Err(message) => return peer_sources_usage_error(&message),
                }
                index += 1;
            }
            arg => {
                return peer_sources_usage_error(&format!("peer-sources: unknown argument {arg}"))
            }
        }
        index += 1;
    }

    if !discovery_rows.is_empty() {
        discoveries = Some(DiscoveryResult::Ok {
            peers: discovery_rows,
        });
    }

    let result = resolve_peer_sources(&config, mode, discoveries.as_ref());
    CliOutput {
        code: 0,
        stdout: if plan_json {
            render_peer_sources_plan_json(&result)
        } else {
            render_peer_sources_plan_text(&result)
        },
        stderr: String::new(),
    }
}

fn peer_sources_usage_error(message: &str) -> CliOutput {
    CliOutput {
        code: 2,
        stdout: String::new(),
        stderr: format!(
            "{message}\nusage: maw-rs peer-sources --mode <config|scout|both> [--peer <url>] [--named-peer <name=url>] [--discovery-ok|--discovery-error <error>] [--discovery-hint <hint>] [--discovered <node|host|oracle|locator[,locator]>]... [--plan-json]\n"
        ),
    }
}

fn parse_discovery_row(value: &str) -> Result<DiscoveryRow, String> {
    let parts: Vec<&str> = value.splitn(4, '|').collect();
    if parts.len() != 4 {
        return Err(
            "peer-sources: --discovered must use <node|host|oracle|locator[,locator]>".to_owned(),
        );
    }
    Ok(DiscoveryRow {
        node: optional_field(parts[0]),
        host: optional_field(parts[1]),
        oracle: optional_field(parts[2]),
        locators: parts[3]
            .split(',')
            .filter(|locator| !locator.is_empty())
            .map(ToOwned::to_owned)
            .collect(),
    })
}

fn optional_field(value: &str) -> Option<String> {
    if value.is_empty() || value == "-" {
        None
    } else {
        Some(value.to_owned())
    }
}

fn render_peer_sources_plan_json(result: &PeerSourceResult) -> String {
    format!(
        "{{\"command\":\"peer-sources\",\"mode\":{},\"peers\":{},\"warnings\":{},\"fetchCalls\":{}}}\n",
        json_string(result.mode.as_str()),
        render_peer_targets_json(result),
        json_string_array(&result.warnings),
        result.fetch_calls
    )
}

fn render_peer_targets_json(result: &PeerSourceResult) -> String {
    format!(
        "[{}]",
        result
            .peers
            .iter()
            .map(|peer| {
                let mut fields = vec![
                    format!("\"url\":{}", json_string(&peer.url)),
                    format!("\"source\":{}", json_string(peer.source.as_str())),
                ];
                push_json_opt(&mut fields, "name", peer.name.as_deref());
                push_json_opt(&mut fields, "node", peer.node.as_deref());
                push_json_opt(&mut fields, "oracle", peer.oracle.as_deref());
                format!("{{{}}}", fields.join(","))
            })
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn render_peer_sources_plan_text(result: &PeerSourceResult) -> String {
    let mut lines = vec![format!(
        "peer-sources mode={} fetchCalls={}",
        result.mode.as_str(),
        result.fetch_calls
    )];
    for peer in &result.peers {
        lines.push(format!(
            "{} {} {}",
            peer.source.as_str(),
            peer.name.as_deref().unwrap_or("-"),
            peer.url
        ));
    }
    for warning in &result.warnings {
        lines.push(format!("warning: {warning}"));
    }
    lines.join("\n") + "\n"
}

#[allow(clippy::too_many_lines)]
fn run_discover_plan(argv: &[String]) -> CliOutput {
    let mut plan_json = false;
    let mut json = false;
    let mut tree = false;
    let mut awake = false;
    let mut peer_source_raw: Option<String> = None;
    let mut config = PeerConfig::default();
    let mut discovery_rows = Vec::new();
    let mut panes = Vec::new();
    let mut inventory_input = DiscoverInventoryInput::default();

    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--plan-json" => plan_json = true,
            "--json" => json = true,
            "--tree" => tree = true,
            "--awake" => awake = true,
            "--peers" => {
                let Some(value) = argv.get(index + 1) else {
                    return discover_usage_error("discover: missing --peers value");
                };
                peer_source_raw = Some(value.to_owned());
                index += 1;
            }
            arg if arg.starts_with("--peers=") => {
                peer_source_raw = Some(arg["--peers=".len()..].to_owned());
            }
            "--peer" => {
                let Some(value) = argv.get(index + 1) else {
                    return discover_usage_error("discover: missing --peer value");
                };
                config.peers.push(value.to_owned());
                index += 1;
            }
            "--named-peer" => {
                let Some(value) = argv.get(index + 1) else {
                    return discover_usage_error("discover: missing --named-peer value");
                };
                match parse_key_value(value, "discover: --named-peer must use <name=url>") {
                    Ok((name, url)) => config.named_peers.push(NamedPeerConfig { name, url }),
                    Err(message) => return discover_usage_error(&message),
                }
                index += 1;
            }
            "--discovered" => {
                let Some(value) = argv.get(index + 1) else {
                    return discover_usage_error("discover: missing --discovered value");
                };
                match parse_discovery_row(value) {
                    Ok(row) => discovery_rows.push(row),
                    Err(message) => {
                        return discover_usage_error(&message.replace("peer-sources", "discover"))
                    }
                }
                index += 1;
            }
            "--pane" => {
                let Some(value) = argv.get(index + 1) else {
                    return discover_usage_error("discover: missing --pane value");
                };
                match parse_discover_pane(value) {
                    Ok(pane) => panes.push(pane),
                    Err(message) => return discover_usage_error(&message),
                }
                index += 1;
            }
            "--plugin" => {
                let Some(value) = argv.get(index + 1) else {
                    return discover_usage_error("discover: missing --plugin value");
                };
                match parse_discover_plugin(value) {
                    Ok(plugin) => inventory_input.plugins.push(plugin),
                    Err(message) => return discover_usage_error(&message),
                }
                index += 1;
            }
            "--ghq" => {
                let Some(value) = argv.get(index + 1) else {
                    return discover_usage_error("discover: missing --ghq value");
                };
                inventory_input.ghq_paths.push(value.to_owned());
                index += 1;
            }
            "--agent" => {
                let Some(value) = argv.get(index + 1) else {
                    return discover_usage_error("discover: missing --agent value");
                };
                match parse_key_value(value, "discover: --agent must use <window=node>") {
                    Ok((window, node)) => {
                        inventory_input.agents.insert(window, node);
                    }
                    Err(message) => return discover_usage_error(&message),
                }
                index += 1;
            }
            "--fleet" => {
                let Some(value) = argv.get(index + 1) else {
                    return discover_usage_error("discover: missing --fleet value");
                };
                match parse_discover_fleet(value) {
                    Ok(record) => inventory_input.fleet.push(record),
                    Err(message) => return discover_usage_error(&message),
                }
                index += 1;
            }
            "--oracle" => {
                let Some(value) = argv.get(index + 1) else {
                    return discover_usage_error("discover: missing --oracle value");
                };
                match parse_discover_oracle(value) {
                    Ok(record) => inventory_input.oracles.push(record),
                    Err(message) => return discover_usage_error(&message),
                }
                index += 1;
            }
            arg => return discover_usage_error(&format!("discover: unknown argument {arg}")),
        }
        index += 1;
    }

    let Some(mode) =
        maw_peer::parse_peer_source_mode(peer_source_raw.as_deref(), PeerSourceMode::Both)
    else {
        return render_discover_invalid_peer_source(plan_json);
    };
    let discoveries = (!discovery_rows.is_empty()).then_some(DiscoveryResult::Ok {
        peers: discovery_rows,
    });
    let result = resolve_peer_sources(&config, mode, discoveries.as_ref());
    let include_live = json || tree || awake;
    let live_probe_calls = usize::from(include_live);
    let live_state = if include_live {
        resolve_tmux_live_state(&result.peers, &panes)
    } else {
        TmuxLiveStateResult {
            source: "tmux".to_owned(),
            live: Vec::new(),
            warnings: Vec::new(),
        }
    };
    let peers_with_live = if include_live {
        mark_peer_targets_live(&result.peers, &live_state.live)
    } else {
        result
            .peers
            .iter()
            .map(peer_with_no_live)
            .collect::<Vec<_>>()
    };
    let visible_peers = if awake && !tree {
        peers_with_live
            .iter()
            .filter(|peer| peer.awake)
            .cloned()
            .collect::<Vec<_>>()
    } else {
        peers_with_live
    };
    let inventory = build_discover_inventory(inventory_input, &visible_peers, &live_state.live);

    CliOutput {
        code: 0,
        stdout: if plan_json || json {
            render_discover_plan_json(
                &result,
                &visible_peers,
                &live_state,
                &inventory,
                tree,
                awake,
                live_probe_calls,
            )
        } else if awake {
            render_discover_live_text(&live_state)
        } else if tree {
            render_discover_tree_text(&visible_peers, &live_state, &inventory)
        } else {
            render_discover_inventory_text(&result, &inventory)
        },
        stderr: String::new(),
    }
}

fn render_discover_invalid_peer_source(plan_json: bool) -> CliOutput {
    let body = "{\"command\":\"discover\",\"ok\":false,\"error\":\"invalid_peer_source\",\"output\":\"usage: maw discover [--peers config|scout|both] [--json] [--tree] [--awake]\",\"fetchCalls\":0,\"liveProbeCalls\":0}\n";
    CliOutput {
        code: if plan_json { 0 } else { 2 },
        stdout: if plan_json {
            body.to_owned()
        } else {
            String::new()
        },
        stderr: if plan_json {
            String::new()
        } else {
            format!("invalid_peer_source\n{}\n", discover_usage())
        },
    }
}

fn discover_usage_error(message: &str) -> CliOutput {
    CliOutput {
        code: 2,
        stdout: String::new(),
        stderr: format!("{message}\n{}\n", discover_usage()),
    }
}

fn discover_usage() -> &'static str {
    "usage: maw-rs discover [--peers config|scout|both] [--peer <url>] [--named-peer <name=url>] [--discovered <node|host|oracle|locator[,locator]>]... [--pane <id|command|target|title|pid|cwd|last_activity>]... [--plugin <name|version|kind|tier|weight|disabled|dir|command|aliases|capabilities|dependencies>] [--ghq <path>] [--agent <window=node>] [--fleet <file|slot|group|session|window|repo>] [--oracle <name|sources|node|session|window|repo|local_path|has_psi|has_fleet_config>] [--json] [--tree] [--awake] [--plan-json]"
}

#[derive(Debug, Clone)]
struct DiscoverPluginRecord {
    name: String,
    version: String,
    kind: String,
    tier: String,
    weight: i64,
    disabled: bool,
    dir: String,
    command: String,
    aliases: Vec<String>,
    capabilities: Vec<String>,
    dependencies: Vec<String>,
}

#[derive(Debug, Clone)]
struct GhqRepoRecord {
    path: String,
    name: String,
    owner: Option<String>,
    host: Option<String>,
    oracle_like: bool,
    worktree: bool,
}

#[derive(Debug, Clone)]
struct FleetConfigRecord {
    file: String,
    slot: String,
    name: String,
    session: String,
    window: String,
    repo: String,
    node: String,
    endpoint: Option<String>,
    peer_matched: bool,
}

#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)]
struct RegisteredOracleRecord {
    name: String,
    sources: Vec<String>,
    node: Option<String>,
    session: Option<String>,
    window: Option<String>,
    repo: Option<String>,
    local_path: Option<String>,
    has_psi: bool,
    has_fleet_config: bool,
    awake: bool,
    ghq_path: Option<String>,
    worktree: bool,
    fleet_matched: bool,
    peer_urls: Vec<String>,
}

#[derive(Debug, Default, Clone)]
struct DiscoverInventoryInput {
    plugins: Vec<DiscoverPluginRecord>,
    ghq_paths: Vec<String>,
    agents: BTreeMap<String, String>,
    fleet: Vec<FleetConfigRecord>,
    oracles: Vec<RegisteredOracleRecord>,
}

#[derive(Debug, Default, Clone)]
struct DiscoverInventory {
    plugins: Vec<DiscoverPluginRecord>,
    ghq: Vec<GhqRepoRecord>,
    fleet: Vec<FleetConfigRecord>,
    oracles: Vec<RegisteredOracleRecord>,
    warnings: Vec<String>,
}

fn parse_discover_pane(value: &str) -> Result<TmuxPane, String> {
    let parts = value.splitn(7, '|').collect::<Vec<_>>();
    if parts.len() != 7 {
        return Err(
            "discover: --pane must use <id|command|target|title|pid|cwd|last_activity>".to_owned(),
        );
    }
    Ok(TmuxPane {
        id: parts[0].to_owned(),
        command: parts[1].to_owned(),
        target: parts[2].to_owned(),
        title: parts[3].to_owned(),
        pid: parse_optional_u32(parts[4], "discover: pane pid must be an integer")?,
        cwd: optional_field(parts[5]),
        last_activity: parse_optional_u64(
            parts[6],
            "discover: pane last_activity must be an integer",
        )?,
    })
}

fn parse_discover_plugin(value: &str) -> Result<DiscoverPluginRecord, String> {
    let parts = value.splitn(11, '|').collect::<Vec<_>>();
    if parts.len() != 11 {
        return Err("discover: --plugin must use <name|version|kind|tier|weight|disabled|dir|command|aliases|capabilities|dependencies>".to_owned());
    }
    Ok(DiscoverPluginRecord {
        name: parts[0].to_owned(),
        version: parts[1].to_owned(),
        kind: parts[2].to_owned(),
        tier: parts[3].to_owned(),
        weight: parts[4]
            .parse::<i64>()
            .map_err(|_| "discover: plugin weight must be an integer".to_owned())?,
        disabled: parse_bool(parts[5], "discover: plugin disabled must be true or false")?,
        dir: parts[6].to_owned(),
        command: parts[7].to_owned(),
        aliases: parse_list_field(parts[8]),
        capabilities: parse_list_field(parts[9]),
        dependencies: parse_list_field(parts[10]),
    })
}

fn parse_discover_fleet(value: &str) -> Result<FleetConfigRecord, String> {
    let parts = value.splitn(6, '|').collect::<Vec<_>>();
    if parts.len() != 6 {
        return Err("discover: --fleet must use <file|slot|group|session|window|repo>".to_owned());
    }
    Ok(FleetConfigRecord {
        file: parts[0].to_owned(),
        slot: parts[1].to_owned(),
        name: parts[2].to_owned(),
        session: parts[3].to_owned(),
        window: parts[4].to_owned(),
        repo: parts[5].to_owned(),
        node: "local".to_owned(),
        endpoint: None,
        peer_matched: false,
    })
}

fn parse_discover_oracle(value: &str) -> Result<RegisteredOracleRecord, String> {
    let parts = value.splitn(9, '|').collect::<Vec<_>>();
    if parts.len() != 9 {
        return Err("discover: --oracle must use <name|sources|node|session|window|repo|local_path|has_psi|has_fleet_config>".to_owned());
    }
    Ok(RegisteredOracleRecord {
        name: parts[0].to_owned(),
        sources: parse_plus_list_field(parts[1]),
        node: optional_field(parts[2]),
        session: optional_field(parts[3]),
        window: optional_field(parts[4]),
        repo: optional_field(parts[5]),
        local_path: optional_field(parts[6]),
        has_psi: parse_bool(parts[7], "discover: oracle has_psi must be true or false")?,
        has_fleet_config: parse_bool(
            parts[8],
            "discover: oracle has_fleet_config must be true or false",
        )?,
        awake: false,
        ghq_path: None,
        worktree: false,
        fleet_matched: false,
        peer_urls: Vec::new(),
    })
}

fn parse_bool(value: &str, message: &str) -> Result<bool, String> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(message.to_owned()),
    }
}

fn parse_list_field(value: &str) -> Vec<String> {
    if value.is_empty() || value == "-" {
        Vec::new()
    } else {
        value.split(',').map(ToOwned::to_owned).collect()
    }
}

fn parse_plus_list_field(value: &str) -> Vec<String> {
    if value.is_empty() || value == "-" {
        Vec::new()
    } else {
        value.split('+').map(ToOwned::to_owned).collect()
    }
}

fn parse_optional_u32(value: &str, message: &str) -> Result<Option<u32>, String> {
    if value.is_empty() || value == "-" {
        return Ok(None);
    }
    value
        .parse::<u32>()
        .map(Some)
        .map_err(|_| message.to_owned())
}

fn parse_optional_u64(value: &str, message: &str) -> Result<Option<u64>, String> {
    if value.is_empty() || value == "-" {
        return Ok(None);
    }
    value
        .parse::<u64>()
        .map(Some)
        .map_err(|_| message.to_owned())
}

fn peer_with_no_live(peer: &maw_peer::PeerTarget) -> PeerTargetWithLive {
    PeerTargetWithLive {
        name: peer.name.clone(),
        url: peer.url.clone(),
        source: peer.source,
        node: peer.node.clone(),
        oracle: peer.oracle.clone(),
        awake: false,
        live_targets: Vec::new(),
        live_sessions: Vec::new(),
    }
}

fn build_discover_inventory(
    mut input: DiscoverInventoryInput,
    peers: &[PeerTargetWithLive],
    live_panes: &[DiscoverLivePane],
) -> DiscoverInventory {
    let mut seen_paths = BTreeSet::new();
    let ghq = input
        .ghq_paths
        .iter()
        .map(|path| path.trim_end_matches('/').replace('\\', "/"))
        .filter(|path| seen_paths.insert(path.to_lowercase()))
        .map(|path| ghq_repo_record(&path))
        .collect::<Vec<_>>();

    let mut seen_fleet = BTreeSet::new();
    let fleet = input
        .fleet
        .drain(..)
        .filter_map(|mut record| {
            record.node = input
                .agents
                .get(&record.window)
                .cloned()
                .unwrap_or_else(|| "local".to_owned());
            if let Some(peer) = peers.iter().find(|peer| {
                peer_matches_name(peer, &record.node)
                    || peer_matches_name(peer, &record.name)
                    || peer_matches_name(peer, &record.window)
            }) {
                record.endpoint = Some(peer.url.clone());
                record.peer_matched = true;
            }
            let key = format!(
                "{}\0{}\0{}",
                record.node.to_lowercase(),
                record.name.to_lowercase(),
                record.repo.to_lowercase()
            );
            seen_fleet.insert(key).then_some(record)
        })
        .collect::<Vec<_>>();

    let mut seen_oracles = BTreeSet::new();
    let oracles = input
        .oracles
        .drain(..)
        .filter_map(|mut oracle| {
            let key = oracle.name.to_lowercase();
            if !seen_oracles.insert(key) {
                return None;
            }
            join_oracle_inventory(&mut oracle, &ghq, &fleet, peers, live_panes);
            Some(oracle)
        })
        .collect::<Vec<_>>();

    DiscoverInventory {
        plugins: input.plugins,
        ghq,
        fleet,
        oracles,
        warnings: Vec::new(),
    }
}

fn ghq_repo_record(path: &str) -> GhqRepoRecord {
    let parts = path
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let name = parts.last().copied().unwrap_or(path).to_owned();
    let host_index = parts.iter().position(|part| part.contains('.'));
    let host = host_index.map(|index| parts[index].to_owned());
    let owner = host_index
        .and_then(|index| parts.get(index + 1))
        .or_else(|| parts.get(parts.len().saturating_sub(2)))
        .map(|owner| (*owner).to_owned());
    GhqRepoRecord {
        path: path.to_owned(),
        oracle_like: is_oracle_like(&name),
        worktree: path.contains(".wt-") || path.contains(".wt/") || path.contains(".wt."),
        name,
        owner,
        host,
    }
}

fn is_oracle_like(name: &str) -> bool {
    name.contains("oracle")
}

fn join_oracle_inventory(
    oracle: &mut RegisteredOracleRecord,
    ghq: &[GhqRepoRecord],
    fleet: &[FleetConfigRecord],
    peers: &[PeerTargetWithLive],
    live_panes: &[DiscoverLivePane],
) {
    if let Some(repo) = ghq.iter().find(|repo| ghq_matches_oracle(repo, oracle)) {
        oracle.ghq_path = Some(repo.path.clone());
        oracle.worktree = repo.worktree;
    }
    oracle.fleet_matched = fleet.iter().any(|record| {
        names_match(&record.name, &oracle.name) || names_match(&record.window, &oracle.name)
    });
    oracle.peer_urls = peers
        .iter()
        .filter(|peer| {
            peer_matches_name(peer, &oracle.name)
                || oracle
                    .node
                    .as_deref()
                    .is_some_and(|node| peer_matches_name(peer, node))
        })
        .map(|peer| peer.url.clone())
        .collect();
    oracle.awake = live_panes
        .iter()
        .any(|pane| pane_matches_oracle(pane, oracle));
}

fn ghq_matches_oracle(repo: &GhqRepoRecord, oracle: &RegisteredOracleRecord) -> bool {
    if oracle.local_path.as_deref() == Some(repo.path.as_str()) {
        return true;
    }
    if oracle.repo.as_deref().is_some_and(|slug| {
        slug.rsplit('/').next() == Some(repo.name.as_str()) || slug.ends_with(&repo.name)
    }) {
        return true;
    }
    names_match(&repo.name, &oracle.name)
}

fn names_match(candidate: &str, name: &str) -> bool {
    let candidate = candidate.to_lowercase();
    let name = name.to_lowercase();
    candidate == name
        || candidate == format!("{name}-oracle")
        || candidate.ends_with(&format!("-{name}"))
}

fn peer_matches_name(peer: &PeerTargetWithLive, name: &str) -> bool {
    peer.name
        .as_deref()
        .is_some_and(|candidate| names_match(candidate, name))
        || peer
            .node
            .as_deref()
            .is_some_and(|candidate| names_match(candidate, name))
        || peer
            .oracle
            .as_deref()
            .is_some_and(|candidate| names_match(candidate, name))
}

fn pane_matches_oracle(pane: &DiscoverLivePane, oracle: &RegisteredOracleRecord) -> bool {
    oracle.session.as_deref() == Some(pane.session.as_str())
        || oracle.window.as_deref() == Some(pane.window.as_str())
        || names_match(&pane.window, &oracle.name)
        || pane
            .matches
            .iter()
            .any(|matched| names_match(matched, &oracle.name))
}

fn render_discover_plan_json(
    result: &PeerSourceResult,
    peers: &[PeerTargetWithLive],
    live_state: &TmuxLiveStateResult,
    inventory: &DiscoverInventory,
    tree: bool,
    awake: bool,
    live_probe_calls: usize,
) -> String {
    let warnings = result
        .warnings
        .iter()
        .chain(live_state.warnings.iter())
        .chain(inventory.warnings.iter())
        .cloned()
        .collect::<Vec<_>>();
    let total = if tree {
        peers.len()
            + live_state.live.len()
            + inventory.fleet.len()
            + inventory.oracles.len()
            + inventory.plugins.len()
            + inventory.ghq.len()
    } else {
        peers.len()
    };
    let tree_field = if tree {
        format!(
            ",\"tree\":{{\"live\":{},\"peers\":{},\"fleet\":{},\"oracles\":{},\"plugins\":{},\"ghq\":{}}}",
            render_live_sessions_json(&live_state.live),
            render_live_peer_targets_json(peers),
            render_fleet_records_json(&inventory.fleet),
            render_oracle_records_json(&inventory.oracles),
            render_plugin_records_json(&inventory.plugins),
            render_ghq_records_json(&inventory.ghq)
        )
    } else {
        String::new()
    };
    format!(
        "{{\"command\":\"discover\",\"ok\":true,\"mode\":{},\"total\":{},\"awake\":{},\"awakeOnly\":{},\"peers\":{},\"fleet\":{{\"source\":\"fleet-config\",\"total\":{},\"records\":{}}},\"oracles\":{{\"source\":\"oracle-manifest\",\"total\":{},\"records\":{}}},\"plugins\":{{\"source\":\"plugin-registry\",\"total\":{},\"records\":{}}},\"ghq\":{{\"source\":\"ghq\",\"total\":{},\"repos\":{}}},\"liveTotal\":{},\"live\":{}{},\"warnings\":{},\"fetchCalls\":{},\"liveProbeCalls\":{}}}\n",
        json_string(result.mode.as_str()),
        total,
        awake,
        awake,
        render_live_peer_targets_json(peers),
        inventory.fleet.len(),
        render_fleet_records_json(&inventory.fleet),
        inventory.oracles.len(),
        render_oracle_records_json(&inventory.oracles),
        inventory.plugins.len(),
        render_plugin_records_json(&inventory.plugins),
        inventory.ghq.len(),
        render_ghq_records_json(&inventory.ghq),
        live_state.live.len(),
        render_live_state_json(live_state),
        tree_field,
        json_string_array(&warnings),
        result.fetch_calls,
        live_probe_calls
    )
}

fn render_plugin_records_json(records: &[DiscoverPluginRecord]) -> String {
    format!(
        "[{}]",
        records
            .iter()
            .map(|record| {
                format!(
                    "{{\"source\":\"plugin-registry\",\"type\":\"plugin\",\"name\":{},\"version\":{},\"kind\":{},\"tier\":{},\"weight\":{},\"disabled\":{},\"dir\":{},\"command\":{},\"aliases\":{},\"capabilities\":{},\"dependencies\":{}}}",
                    json_string(&record.name),
                    json_string(&record.version),
                    json_string(&record.kind),
                    json_string(&record.tier),
                    record.weight,
                    record.disabled,
                    json_string(&record.dir),
                    json_string(&record.command),
                    json_string_array(&record.aliases),
                    json_string_array(&record.capabilities),
                    json_string_array(&record.dependencies)
                )
            })
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn render_ghq_records_json(records: &[GhqRepoRecord]) -> String {
    format!(
        "[{}]",
        records
            .iter()
            .map(|record| {
                format!(
                    "{{\"source\":\"ghq\",\"type\":\"repo\",\"path\":{},\"name\":{},\"owner\":{},\"host\":{},\"oracleLike\":{},\"worktree\":{}}}",
                    json_string(&record.path),
                    json_string(&record.name),
                    json_opt_string(record.owner.as_deref()),
                    json_opt_string(record.host.as_deref()),
                    record.oracle_like,
                    record.worktree
                )
            })
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn render_fleet_records_json(records: &[FleetConfigRecord]) -> String {
    format!(
        "[{}]",
        records
            .iter()
            .map(|record| {
                format!(
                    "{{\"source\":\"fleet-config\",\"type\":\"workspace\",\"file\":{},\"slot\":{},\"name\":{},\"session\":{},\"window\":{},\"repo\":{},\"node\":{},\"endpoint\":{},\"peerMatched\":{}}}",
                    json_string(&record.file),
                    json_string(&record.slot),
                    json_string(&record.name),
                    json_string(&record.session),
                    json_string(&record.window),
                    json_string(&record.repo),
                    json_string(&record.node),
                    json_opt_string(record.endpoint.as_deref()),
                    record.peer_matched
                )
            })
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn render_oracle_records_json(records: &[RegisteredOracleRecord]) -> String {
    format!(
        "[{}]",
        records
            .iter()
            .map(|record| {
                format!(
                    "{{\"source\":\"oracle-manifest\",\"type\":\"oracle\",\"name\":{},\"sources\":{},\"node\":{},\"session\":{},\"window\":{},\"repo\":{},\"localPath\":{},\"hasPsi\":{},\"hasFleetConfig\":{},\"awake\":{},\"ghqPath\":{},\"worktree\":{},\"fleetMatched\":{},\"peerUrls\":{}}}",
                    json_string(&record.name),
                    json_string_array(&record.sources),
                    json_opt_string(record.node.as_deref()),
                    json_opt_string(record.session.as_deref()),
                    json_opt_string(record.window.as_deref()),
                    json_opt_string(record.repo.as_deref()),
                    json_opt_string(record.local_path.as_deref()),
                    record.has_psi,
                    record.has_fleet_config,
                    record.awake,
                    json_opt_string(record.ghq_path.as_deref()),
                    record.worktree,
                    record.fleet_matched,
                    json_string_array(&record.peer_urls)
                )
            })
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn render_discover_inventory_text(
    result: &PeerSourceResult,
    inventory: &DiscoverInventory,
) -> String {
    let mut output = render_peer_sources_plan_text(result);
    if !inventory.oracles.is_empty() {
        output.push_str("registered oracles\n");
        for oracle in &inventory.oracles {
            let status = if oracle.awake { "awake" } else { "offline" };
            let _ = writeln!(output, "{} {}", oracle.name, status);
        }
    }
    if !inventory.fleet.is_empty() {
        output.push_str("fleet config\n");
        for record in &inventory.fleet {
            let _ = writeln!(output, "{} {} {}", record.name, record.node, record.repo);
        }
    }
    if !inventory.plugins.is_empty() {
        output.push_str("plugin registry\n");
        for plugin in &inventory.plugins {
            let status = if plugin.disabled {
                "disabled"
            } else {
                "enabled"
            };
            let _ = writeln!(output, "{} {} {}", plugin.name, plugin.version, status);
        }
    }
    if !inventory.ghq.is_empty() {
        output.push_str("ghq repos\n");
        for repo in &inventory.ghq {
            let _ = writeln!(output, "{} {}", repo.name, repo.path);
        }
    }
    output
}

fn render_discover_tree_text(
    peers: &[PeerTargetWithLive],
    live_state: &TmuxLiveStateResult,
    inventory: &DiscoverInventory,
) -> String {
    let mut output = String::new();
    let _ = writeln!(output, "discover tree");
    let _ = writeln!(output, "live ({} sessions)", live_state.live.len());
    let _ = writeln!(output, "peers ({} configured)", peers.len());
    let _ = writeln!(
        output,
        "fleet config ({} configured)",
        inventory.fleet.len()
    );
    let _ = writeln!(output, "registered oracles ({})", inventory.oracles.len());
    let _ = writeln!(output, "plugins ({} registered)", inventory.plugins.len());
    for plugin in &inventory.plugins {
        let _ = writeln!(output, "  - {}", plugin.name);
    }
    let _ = writeln!(output, "ghq ({} repos)", inventory.ghq.len());
    for repo in &inventory.ghq {
        let _ = writeln!(output, "  - {}", repo.path);
    }
    output
}

fn render_live_peer_targets_json(peers: &[PeerTargetWithLive]) -> String {
    format!(
        "[{}]",
        peers
            .iter()
            .map(|peer| {
                let mut fields = vec![
                    format!("\"url\":{}", json_string(&peer.url)),
                    format!("\"source\":{}", json_string(peer.source.as_str())),
                ];
                push_json_opt(&mut fields, "name", peer.name.as_deref());
                push_json_opt(&mut fields, "node", peer.node.as_deref());
                push_json_opt(&mut fields, "oracle", peer.oracle.as_deref());
                fields.push(format!("\"awake\":{}", peer.awake));
                fields.push(format!(
                    "\"liveTargets\":{}",
                    json_string_array(&peer.live_targets)
                ));
                fields.push(format!(
                    "\"liveSessions\":{}",
                    json_string_array(&peer.live_sessions)
                ));
                format!("{{{}}}", fields.join(","))
            })
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn render_live_state_json(live_state: &TmuxLiveStateResult) -> String {
    format!(
        "{{\"source\":{},\"total\":{},\"panes\":{},\"sessions\":{}}}",
        json_string(&live_state.source),
        live_state.live.len(),
        render_live_panes_json(&live_state.live),
        render_live_sessions_json(&live_state.live)
    )
}

fn render_live_panes_json(panes: &[DiscoverLivePane]) -> String {
    format!(
        "[{}]",
        panes
            .iter()
            .map(|pane| {
                let mut fields = vec![
                    format!("\"source\":{}", json_string(&pane.source)),
                    format!("\"id\":{}", json_string(&pane.id)),
                    format!("\"target\":{}", json_string(&pane.target)),
                    format!("\"session\":{}", json_string(&pane.session)),
                    format!("\"window\":{}", json_string(&pane.window)),
                    format!("\"pane\":{}", json_string(&pane.pane)),
                    format!("\"awake\":{}", pane.awake),
                    format!("\"matches\":{}", json_string_array(&pane.matches)),
                ];
                push_json_opt(&mut fields, "command", pane.command.as_deref());
                push_json_opt(&mut fields, "title", pane.title.as_deref());
                if let Some(pid) = pane.pid {
                    fields.push(format!("\"pid\":{pid}"));
                }
                push_json_opt(&mut fields, "cwd", pane.cwd.as_deref());
                if let Some(last_activity) = pane.last_activity {
                    fields.push(format!("\"lastActivity\":{last_activity}"));
                }
                format!("{{{}}}", fields.join(","))
            })
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn render_live_sessions_json(panes: &[DiscoverLivePane]) -> String {
    let mut sessions: BTreeMap<&str, BTreeMap<&str, Vec<&DiscoverLivePane>>> = BTreeMap::new();
    for pane in panes {
        sessions
            .entry(&pane.session)
            .or_default()
            .entry(&pane.window)
            .or_default()
            .push(pane);
    }
    format!(
        "[{}]",
        sessions
            .into_iter()
            .map(|(name, windows)| {
                let pane_count = windows.values().map(Vec::len).sum::<usize>();
                let windows_json = windows
                    .into_iter()
                    .map(|(window_name, window_panes)| {
                        let cloned = window_panes.into_iter().cloned().collect::<Vec<_>>();
                        format!(
                            "{{\"name\":{},\"paneCount\":{},\"panes\":{}}}",
                            json_string(window_name),
                            cloned.len(),
                            render_live_panes_json(&cloned)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                format!(
                    "{{\"source\":\"tmux\",\"name\":{},\"awake\":true,\"paneCount\":{},\"windows\":[{}]}}",
                    json_string(name),
                    pane_count,
                    windows_json
                )
            })
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn render_discover_live_text(live_state: &TmuxLiveStateResult) -> String {
    if live_state.live.is_empty() {
        return "no live tmux sessions/windows found\n".to_owned();
    }
    live_state
        .live
        .iter()
        .map(|pane| {
            format!(
                "tmux {} {}",
                pane.target,
                pane.command.as_deref().unwrap_or("-")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

#[allow(clippy::too_many_lines)]
fn run_route_plan(argv: &[String]) -> CliOutput {
    let mut plan_json = false;
    let mut query = None;
    let mut config = RouteConfig::default();
    let mut sessions: Vec<RouteSession> = Vec::new();
    let mut current_session: Option<RouteSession> = None;

    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--plan-json" => plan_json = true,
            "--query" => {
                let Some(value) = argv.get(index + 1) else {
                    return route_usage_error("route: missing --query value");
                };
                query = Some(value.to_owned());
                index += 1;
            }
            "--node" => {
                let Some(value) = argv.get(index + 1) else {
                    return route_usage_error("route: missing --node value");
                };
                config.node = Some(value.to_owned());
                index += 1;
            }
            "--named-peer" => {
                let Some(value) = argv.get(index + 1) else {
                    return route_usage_error("route: missing --named-peer value");
                };
                match parse_key_value(value, "route: --named-peer must use <name=url>") {
                    Ok((name, url)) => config.named_peers.push(RouteNamedPeer { name, url }),
                    Err(message) => return route_usage_error(&message),
                }
                index += 1;
            }
            "--peer" => {
                let Some(value) = argv.get(index + 1) else {
                    return route_usage_error("route: missing --peer value");
                };
                config.peers.push(value.to_owned());
                index += 1;
            }
            "--agent" => {
                let Some(value) = argv.get(index + 1) else {
                    return route_usage_error("route: missing --agent value");
                };
                match parse_key_value(value, "route: --agent must use <agent=node>") {
                    Ok((agent, node)) => {
                        config.agents.insert(agent, node);
                    }
                    Err(message) => return route_usage_error(&message),
                }
                index += 1;
            }
            "--session" => {
                if let Some(session) = current_session.take() {
                    sessions.push(session);
                }
                let Some(value) = argv.get(index + 1) else {
                    return route_usage_error("route: missing --session value");
                };
                current_session = Some(RouteSession {
                    name: value.to_owned(),
                    windows: Vec::new(),
                    source: None,
                });
                index += 1;
            }
            "--source" => {
                let Some(value) = argv.get(index + 1) else {
                    return route_usage_error("route: missing --source value");
                };
                let Some(session) = &mut current_session else {
                    return route_usage_error("route: --source must follow a --session");
                };
                session.source = Some(value.to_owned());
                index += 1;
            }
            "--window" => {
                let Some(value) = argv.get(index + 1) else {
                    return route_usage_error("route: missing --window value");
                };
                let Some(session) = &mut current_session else {
                    return route_usage_error("route: --window must follow a --session");
                };
                match parse_route_window(value) {
                    Ok(window) => session.windows.push(window),
                    Err(message) => return route_usage_error(&message),
                }
                index += 1;
            }
            arg => return route_usage_error(&format!("route: unknown argument {arg}")),
        }
        index += 1;
    }
    if let Some(session) = current_session.take() {
        sessions.push(session);
    }

    let Some(query) = query else {
        return route_usage_error("route: expected --query <target>");
    };
    let result = resolve_route_target(&query, &config, &sessions);
    CliOutput {
        code: 0,
        stdout: if plan_json {
            render_route_plan_json(&query, &result)
        } else {
            render_route_plan_text(&query, &result)
        },
        stderr: String::new(),
    }
}

fn route_usage_error(message: &str) -> CliOutput {
    CliOutput {
        code: 2,
        stdout: String::new(),
        stderr: format!(
            "{message}\nusage: maw-rs route --query <target> [--node <name>] [--named-peer <name=url>] [--peer <url>] [--agent <agent=node>] [--session <name>] [--source <source>] [--window <index:name:active>]... [--plan-json]\n"
        ),
    }
}

fn parse_key_value(value: &str, message: &str) -> Result<(String, String), String> {
    let Some((key, value)) = value.split_once('=') else {
        return Err(message.to_owned());
    };
    if key.is_empty() || value.is_empty() {
        return Err(message.to_owned());
    }
    Ok((key.to_owned(), value.to_owned()))
}

fn parse_route_window(value: &str) -> Result<RouteWindow, String> {
    let mut parts = value.splitn(3, ':');
    let index = parts
        .next()
        .ok_or_else(|| "route: missing window index".to_owned())?
        .parse::<u32>()
        .map_err(|_| "route: invalid window index".to_owned())?;
    let Some(name) = parts.next() else {
        return Err("route: window must use <index:name:active>".to_owned());
    };
    let active = match parts.next() {
        Some("true") => true,
        Some("false") => false,
        _ => return Err("route: window active must be true or false".to_owned()),
    };
    Ok(RouteWindow {
        index,
        name: name.to_owned(),
        active,
    })
}

fn render_route_plan_json(query: &str, result: &RouteResult) -> String {
    let mut fields = vec![
        "\"command\":\"route\"".to_owned(),
        format!("\"query\":{}", json_string(query)),
    ];
    match result {
        RouteResult::Local { target } => {
            fields.push("\"type\":\"local\"".to_owned());
            fields.push(format!("\"target\":{}", json_string(target)));
        }
        RouteResult::Peer {
            peer_url,
            target,
            node,
        } => {
            fields.push("\"type\":\"peer\"".to_owned());
            fields.push(format!("\"peerUrl\":{}", json_string(peer_url)));
            fields.push(format!("\"target\":{}", json_string(target)));
            fields.push(format!("\"node\":{}", json_string(node)));
        }
        RouteResult::SelfNode { target } => {
            fields.push("\"type\":\"self-node\"".to_owned());
            fields.push(format!("\"target\":{}", json_string(target)));
        }
        RouteResult::Error {
            reason,
            detail,
            hint,
        } => {
            fields.push("\"type\":\"error\"".to_owned());
            fields.push(format!("\"reason\":{}", json_string(reason)));
            fields.push(format!("\"detail\":{}", json_string(detail)));
            if let Some(hint) = hint {
                fields.push(format!("\"hint\":{}", json_string(hint)));
            }
        }
    }
    format!("{{{}}}\n", fields.join(","))
}

fn render_route_plan_text(query: &str, result: &RouteResult) -> String {
    match result {
        RouteResult::Local { target } => format!("route {query}: local {target}\n"),
        RouteResult::Peer {
            peer_url,
            target,
            node,
        } => format!("route {query}: peer {node} {target} via {peer_url}\n"),
        RouteResult::SelfNode { target } => format!("route {query}: self-node {target}\n"),
        RouteResult::Error {
            reason,
            detail,
            hint,
        } => hint.as_ref().map_or_else(
            || format!("route {query}: error {reason} {detail}\n"),
            |hint| format!("route {query}: error {reason} {detail} hint={hint}\n"),
        ),
    }
}

fn run_worktree_window_plan(argv: &[String]) -> CliOutput {
    let mut plan_json = false;
    let mut main_repo_name = None;
    let mut wt_name = None;
    let mut sessions: Vec<WorktreeSession> = Vec::new();
    let mut current_session: Option<WorktreeSession> = None;

    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--plan-json" => plan_json = true,
            "--main-repo-name" => {
                let Some(value) = argv.get(index + 1) else {
                    return worktree_window_usage_error(
                        "worktree-window: missing --main-repo-name value",
                    );
                };
                main_repo_name = Some(value.to_owned());
                index += 1;
            }
            "--wt-name" => {
                let Some(value) = argv.get(index + 1) else {
                    return worktree_window_usage_error("worktree-window: missing --wt-name value");
                };
                wt_name = Some(value.to_owned());
                index += 1;
            }
            "--session" => {
                if let Some(session) = current_session.take() {
                    sessions.push(session);
                }
                let Some(value) = argv.get(index + 1) else {
                    return worktree_window_usage_error("worktree-window: missing --session value");
                };
                current_session = Some(WorktreeSession {
                    name: value.to_owned(),
                    windows: Vec::new(),
                });
                index += 1;
            }
            "--window" => {
                let Some(value) = argv.get(index + 1) else {
                    return worktree_window_usage_error("worktree-window: missing --window value");
                };
                let Some(session) = &mut current_session else {
                    return worktree_window_usage_error(
                        "worktree-window: --window must follow a --session",
                    );
                };
                match parse_worktree_window(value) {
                    Ok(window) => session.windows.push(window),
                    Err(message) => return worktree_window_usage_error(&message),
                }
                index += 1;
            }
            arg => {
                return worktree_window_usage_error(&format!(
                    "worktree-window: unknown argument {arg}"
                ));
            }
        }
        index += 1;
    }
    if let Some(session) = current_session.take() {
        sessions.push(session);
    }

    let Some(main_repo_name) = main_repo_name else {
        return worktree_window_usage_error("worktree-window: expected --main-repo-name <repo>");
    };
    let Some(wt_name) = wt_name else {
        return worktree_window_usage_error("worktree-window: expected --wt-name <worktree>");
    };

    let result = resolve_worktree_window(&main_repo_name, &wt_name, &sessions);
    CliOutput {
        code: 0,
        stdout: if plan_json {
            render_worktree_window_plan_json(&main_repo_name, &wt_name, &result)
        } else {
            render_worktree_window_plan_text(&main_repo_name, &wt_name, &result)
        },
        stderr: String::new(),
    }
}

fn worktree_window_usage_error(message: &str) -> CliOutput {
    CliOutput {
        code: 2,
        stdout: String::new(),
        stderr: format!(
            "{message}\nusage: maw-rs worktree-window --main-repo-name <repo> --wt-name <worktree> [--session <name>] [--window <index:name:active>]... [--plan-json]\n"
        ),
    }
}

fn parse_worktree_window(value: &str) -> Result<WorktreeWindow, String> {
    let mut parts = value.splitn(3, ':');
    let index = parts
        .next()
        .ok_or_else(|| "worktree-window: missing window index".to_owned())?
        .parse::<u32>()
        .map_err(|_| "worktree-window: invalid window index".to_owned())?;
    let Some(name) = parts.next() else {
        return Err("worktree-window: window must use <index:name:active>".to_owned());
    };
    let active = match parts.next() {
        Some("true") => true,
        Some("false") => false,
        _ => return Err("worktree-window: window active must be true or false".to_owned()),
    };
    Ok(WorktreeWindow {
        index,
        name: name.to_owned(),
        active,
    })
}

fn render_worktree_window_plan_json(
    main_repo_name: &str,
    wt_name: &str,
    result: &WorktreeWindowResolution,
) -> String {
    let mut fields = vec![
        "\"command\":\"worktree-window\"".to_owned(),
        format!("\"mainRepoName\":{}", json_string(main_repo_name)),
        format!("\"wtName\":{}", json_string(wt_name)),
    ];
    match result {
        WorktreeWindowResolution::Bound { window } => {
            fields.push("\"kind\":\"bound\"".to_owned());
            fields.push(format!("\"window\":{}", json_string(window)));
        }
        WorktreeWindowResolution::Ambiguous { query, candidates } => {
            fields.push("\"kind\":\"ambiguous\"".to_owned());
            fields.push(format!("\"query\":{}", json_string(query)));
            fields.push(format!("\"candidates\":{}", json_string_array(candidates)));
        }
        WorktreeWindowResolution::None => fields.push("\"kind\":\"none\"".to_owned()),
    }
    format!("{{{}}}\n", fields.join(","))
}

fn render_worktree_window_plan_text(
    main_repo_name: &str,
    wt_name: &str,
    result: &WorktreeWindowResolution,
) -> String {
    match result {
        WorktreeWindowResolution::Bound { window } => {
            format!("worktree-window {main_repo_name} {wt_name}: bound {window}\n")
        }
        WorktreeWindowResolution::Ambiguous { query, candidates } => format!(
            "worktree-window {main_repo_name} {wt_name}: ambiguous {query} candidates={}\n",
            candidates.join(", ")
        ),
        WorktreeWindowResolution::None => {
            format!("worktree-window {main_repo_name} {wt_name}: none\n")
        }
    }
}

fn run_calver_plan(argv: &[String]) -> CliOutput {
    let mut plan_json = false;
    let mut stable = false;
    let mut channel = None;
    let mut now = None;
    let mut package_version = String::new();
    let mut tags = Vec::new();

    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--plan-json" => plan_json = true,
            "--stable" => stable = true,
            "--alpha" => channel = Some(Channel::Alpha),
            "--beta" => channel = Some(Channel::Beta),
            "--now" => {
                let Some(value) = argv.get(index + 1) else {
                    return calver_usage_error("calver: missing --now value");
                };
                match parse_date_parts(value) {
                    Ok(parsed) => now = Some(parsed),
                    Err(message) => return calver_usage_error(&message),
                }
                index += 1;
            }
            "--package-version" => {
                let Some(value) = argv.get(index + 1) else {
                    return calver_usage_error("calver: missing --package-version value");
                };
                package_version.clone_from(value);
                index += 1;
            }
            "--tag" => {
                let Some(value) = argv.get(index + 1) else {
                    return calver_usage_error("calver: missing --tag value");
                };
                tags.push(value.to_owned());
                index += 1;
            }
            arg => return calver_usage_error(&format!("calver: unknown argument {arg}")),
        }
        index += 1;
    }

    let Some(now) = now else {
        return calver_usage_error("calver: expected --now <YYYY-M-DTHH:MM>");
    };

    let compute_args = ComputeArgs {
        stable,
        channel,
        now,
    };
    match compute_version(compute_args, &tags, &package_version) {
        Ok(version) => CliOutput {
            code: 0,
            stdout: if plan_json {
                render_calver_plan_json(compute_args, &tags, &package_version, &version)
            } else {
                format!("{version}\n")
            },
            stderr: String::new(),
        },
        Err(error) => CliOutput {
            code: 1,
            stdout: String::new(),
            stderr: format!("calver: {error}\n"),
        },
    }
}

fn calver_usage_error(message: &str) -> CliOutput {
    CliOutput {
        code: 2,
        stdout: String::new(),
        stderr: format!(
            "{message}\nusage: maw-rs calver --now <YYYY-M-DTHH:MM> [--stable|--alpha|--beta] [--package-version <version>] [--tag <tag>]... [--plan-json]\n"
        ),
    }
}

fn parse_date_parts(value: &str) -> Result<DateParts, String> {
    let Some((date, time)) = value.split_once('T') else {
        return Err("calver: --now must use YYYY-M-DTHH:MM".to_owned());
    };
    let mut date_parts = date.split('-');
    let year = parse_i32_part(date_parts.next(), "year")?;
    let month = parse_u32_part(date_parts.next(), "month")?;
    let day = parse_u32_part(date_parts.next(), "day")?;
    if date_parts.next().is_some() {
        return Err("calver: --now date must use YYYY-M-D".to_owned());
    }

    let mut time_parts = time.split(':');
    let hour = parse_u32_part(time_parts.next(), "hour")?;
    let minute = parse_u32_part(time_parts.next(), "minute")?;
    if time_parts.next().is_some() {
        return Err("calver: --now time must use HH:MM".to_owned());
    }
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 {
        return Err("calver: --now contains out-of-range date/time parts".to_owned());
    }
    Ok(DateParts {
        year,
        month,
        day,
        hour,
        minute,
    })
}

fn parse_i32_part(value: Option<&str>, name: &str) -> Result<i32, String> {
    let Some(value) = value else {
        return Err(format!("calver: missing {name} in --now"));
    };
    value
        .parse::<i32>()
        .map_err(|_| format!("calver: invalid {name} in --now"))
}

fn parse_u32_part(value: Option<&str>, name: &str) -> Result<u32, String> {
    let Some(value) = value else {
        return Err(format!("calver: missing {name} in --now"));
    };
    value
        .parse::<u32>()
        .map_err(|_| format!("calver: invalid {name} in --now"))
}

fn render_calver_plan_json(
    args: ComputeArgs,
    tags: &[String],
    package_version: &str,
    version: &str,
) -> String {
    let mut arg_fields = vec![
        format!("\"stable\":{}", args.stable),
        format!("\"now\":{}", render_date_parts_json(args.now)),
    ];
    if let Some(channel) = args.channel {
        arg_fields.push(format!("\"channel\":{}", json_string(channel.as_str())));
    }
    format!(
        "{{\"command\":\"calver\",\"args\":{{{}}},\"tags\":{},\"packageVersion\":{},\"version\":{}}}\n",
        arg_fields.join(","),
        json_string_array(tags),
        json_string(package_version),
        json_string(version)
    )
}

fn render_date_parts_json(now: DateParts) -> String {
    format!(
        "{{\"year\":{},\"month\":{},\"day\":{},\"hour\":{},\"minute\":{}}}",
        now.year, now.month, now.day, now.hour, now.minute
    )
}

fn run_normalize_plan(argv: &[String]) -> CliOutput {
    let plan_json = argv.iter().any(|arg| arg == "--plan-json");
    let target = argv.iter().find(|arg| arg.as_str() != "--plan-json");
    let Some(target) = target else {
        return CliOutput {
            code: 2,
            stdout: String::new(),
            stderr:
                "normalize: expected <target>\nusage: maw-rs normalize <target> [--plan-json]\n"
                    .to_owned(),
        };
    };
    let normalized = normalize_target(target);
    CliOutput {
        code: 0,
        stdout: if plan_json {
            format!(
                "{{\"command\":\"normalize\",\"input\":{},\"normalized\":{}}}\n",
                json_string(target),
                json_string(&normalized)
            )
        } else {
            format!("{normalized}\n")
        },
        stderr: String::new(),
    }
}

fn run_resolve_plan(argv: &[String]) -> CliOutput {
    let plan_json = argv.iter().any(|arg| arg == "--plan-json");
    let mut mode = "by-name".to_owned();
    let mut positionals = Vec::new();
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--plan-json" => {}
            "--mode" => {
                let Some(value) = argv.get(index + 1) else {
                    return resolve_usage_error("resolve: missing --mode value");
                };
                mode.clone_from(value);
                index += 1;
            }
            arg => positionals.push(arg.to_owned()),
        }
        index += 1;
    }

    if positionals.len() < 2 {
        return resolve_usage_error("resolve: expected <target> and at least one item");
    }
    let target = &positionals[0];
    let items = &positionals[1..];
    let result = match mode.as_str() {
        "by-name" | "byName" => resolve_by_name(target, items, ResolveOptions::default()),
        "session" => resolve_session_target(target, items),
        "worktree" => resolve_worktree_target(target, items),
        _ => return resolve_usage_error("resolve: unknown --mode"),
    };
    let stdout = if plan_json {
        render_resolve_plan_json(&mode, target, result)
    } else {
        render_resolve_plan_text(&mode, target, result)
    };
    CliOutput {
        code: 0,
        stdout,
        stderr: String::new(),
    }
}

fn resolve_usage_error(message: &str) -> CliOutput {
    CliOutput {
        code: 2,
        stdout: String::new(),
        stderr: format!("{message}\nusage: maw-rs resolve --mode <by-name|session|worktree> <target> <item...> [--plan-json]\n"),
    }
}

fn render_resolve_plan_json(mode: &str, target: &str, result: ResolveResult<String>) -> String {
    let mut fields = vec![
        "\"command\":\"resolve\"".to_owned(),
        format!("\"mode\":{}", json_string(mode)),
        format!("\"target\":{}", json_string(target)),
    ];
    match result {
        ResolveResult::Exact { matched } => {
            fields.push("\"kind\":\"exact\"".to_owned());
            fields.push(format!("\"match\":{}", json_string(&matched)));
        }
        ResolveResult::Fuzzy { matched } => {
            fields.push("\"kind\":\"fuzzy\"".to_owned());
            fields.push(format!("\"match\":{}", json_string(&matched)));
        }
        ResolveResult::Ambiguous { candidates } => {
            fields.push("\"kind\":\"ambiguous\"".to_owned());
            fields.push(format!("\"candidates\":{}", json_string_array(&candidates)));
        }
        ResolveResult::None { hints } => {
            fields.push("\"kind\":\"none\"".to_owned());
            if let Some(hints) = hints {
                fields.push(format!("\"hints\":{}", json_string_array(&hints)));
            }
        }
    }
    format!("{{{}}}\n", fields.join(","))
}

fn render_resolve_plan_text(mode: &str, target: &str, result: ResolveResult<String>) -> String {
    match result {
        ResolveResult::Exact { matched } => {
            format!("resolve {mode} {target}: exact {matched}\n")
        }
        ResolveResult::Fuzzy { matched } => {
            format!("resolve {mode} {target}: fuzzy {matched}\n")
        }
        ResolveResult::Ambiguous { candidates } => {
            format!(
                "resolve {mode} {target}: ambiguous {}\n",
                candidates.join(", ")
            )
        }
        ResolveResult::None { hints } => hints.map_or_else(
            || format!("resolve {mode} {target}: none\n"),
            |hints| format!("resolve {mode} {target}: none hints={}\n", hints.join(", ")),
        ),
    }
}

fn json_string_array(values: &[String]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|value| json_string(value))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn json_str_array(values: &[&str]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|value| json_string(value))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn usage_ok() -> CliOutput {
    CliOutput {
        code: 0,
        stdout: usage_text(),
        stderr: String::new(),
    }
}

fn usage_text() -> String {
    "usage: maw-rs <command> [args]\ncommands:\n  auto-wake <target> --site <view|hey|api-send|api-wake|peek|bud|wake-cmd> [--fleet-known|--unknown-fleet] [--live|--not-live] [--wake] [--no-wake] [--canonical-target] [--manifest-source <source>]... [--manifest-live <true|false>] [--plan-json]
  auth sign-v3 --peer-key <hex> --from <addr> [--method <method>] [--path <path>] [--now <ts>] [--body <body>] [--plan-json]\n  auth verify-request [--method <method>] [--path <path>] [--now <ts>] [--body <body>] [--cached-pubkey <hex>] [--header <KEY=VALUE>]... [--plan-json]\n  hub validate-workspace --name <name> --url <url> [--plan-json]\n  hub load-workspaces --dir <dir> [--plan-json]\n  xdg paths [--home <dir>] [--env <KEY=VALUE>]... [--plan-json]\n  xdg core-paths [--home <dir>] [--env <KEY=VALUE>]... [--plan-json]\n  xdg validate-instance --name <name> [--plan-json]\n  plugin-scaffold validate-name --name <name> [--plan-json]\n  plugin-scaffold manifest --name <name> (--rust|--as) [--plan-json]\n  plugin-manifest parse --dir <dir> --json <json> [--plan-json]\n  plugin-manifest load --dir <dir> [--plan-json]\n  plugin-manifest discover --scan-dir <dir>... [--disabled <name>]... [--runtime-version <version>] [--use-cache] [--plan-json]\n  plugin-manifest import-symbol --scan-dir <dir>... --plugin <name> --symbol <name> [--module-symbol <name=value>]... [--disabled <name>]... [--runtime-version <version>] [--plan-json]\n  plugin-manifest invoke --scan-dir <dir>... --plugin <name> [--source <cli|api|peer>] [--arg <arg>]... [--fake-ts-output <text>] [--fake-wasm-output <text>] [--disabled <name>]... [--runtime-version <version>] [--plan-json]\n  bind-host [--config-peers-len <n>] [--config-named-peers-len <n>] [--maw-host <host>] [--peers-store-len <n>|--peers-store-error <err>] [--plan-json]\n  bring|b <oracle> [--to <session[:window]>] [--plan-json]\n  feed parse-line <line> [--plan-json]\n  feed describe <event> [--message <message>] [--plan-json]\n  feed active --now <ms> --window <ms> [--event <oracle:ts:message>]... [--plan-json]\n  fuzzy distance <left> <right> [--plan-json]\n  fuzzy match <input> [--candidate <candidate>]... [--max-results <n>] [--max-distance <n>] [--plan-json]\n  resolve --mode <by-name|session|worktree> <target> <item...> [--plan-json]\n  identity session-name <oracle> [--slot <0-99>] [--plan-json]\n  identity node-identity <host> [--user <user>] [--plan-json]\n  normalize <target> [--plan-json]\n  calver --now <YYYY-M-DTHH:MM> [--stable|--alpha|--beta] [--package-version <version>] [--tag <tag>]... [--plan-json]\n  worktree-window --main-repo-name <repo> --wt-name <worktree> [--session <name>] [--window <index:name:active>]... [--plan-json]\n  route --query <target> [--node <name>] [--named-peer <name=url>] [--peer <url>] [--agent <agent=node>] [--session <name>] [--source <source>] [--window <index:name:active>]... [--plan-json]\n  discover [--peers config|scout|both] [--peer <url>] [--named-peer <name=url>] [--discovered <node|host|oracle|locator[,locator]>]... [--pane <id|command|target|title|pid|cwd|last_activity>]... [--json] [--tree] [--awake] [--plan-json]\n  peer-probe classify (--http-status <n>|--code <code>|--cause-code <code>|--name <name>|--non-object) [--plan-json]
  peer-probe format --code <code> --message <msg> --url <url> --alias <alias> [--at <ts>] [--plan-json]
  peer-probe handshake (--legacy-true|--schema <schema>|--empty-object|--other-truthy|--missing) [--plan-json]
  peer-sources --mode <config|scout|both> [--peer <url>] [--named-peer <name=url>] [--discovery-ok|--discovery-error <error>] [--discovery-hint <hint>] [--discovered <node|host|oracle|locator[,locator]>]... [--plan-json]\n  policy [--constants|--weight <i32>|--default-active <key> [--includes <plugin>]] [--plan-json]\n  split-policy [--pane-current-command <cmd>] [--requested-policy <policy>] [--no-attach] [--force-split] [--plan-json]\n  transport --classify-error <error>|--classify-empty|--send [--transport <name[:connected][:canReach][:ok|false|throw=err]>]... [--plan-json]\n"
        .to_owned()
}

fn run_bring_plan(argv: &[String]) -> CliOutput {
    let plan_json = argv.iter().any(|arg| arg == "--plan-json");
    let filtered: Vec<String> = argv
        .iter()
        .filter(|arg| arg.as_str() != "--plan-json")
        .cloned()
        .collect();
    match parse_bring_args(&filtered) {
        Ok(parsed) => CliOutput {
            code: 0,
            stdout: if plan_json {
                render_bring_plan_json(&parsed)
            } else {
                render_bring_plan_text(&parsed)
            },
            stderr: String::new(),
        },
        Err(error) => CliOutput {
            code: 2,
            stdout: String::new(),
            stderr: format!("{}\n{}\n", error.message, error.usage.join("\n")),
        },
    }
}

fn render_bring_plan_text(parsed: &ParsedBringArgs) -> String {
    let mut lines = vec![format!("wake {} --split", parsed.oracle)];
    if let Some(engine) = &parsed.opts.engine {
        lines.push(format!("engine: {engine}"));
    }
    if let Some(session) = &parsed.opts.session {
        lines.push(format!("session: {session}"));
    }
    if let Some(split_target) = &parsed.opts.split_target {
        lines.push(format!("split-target: {split_target}"));
    }
    if parsed.opts.pick {
        lines.push("pick: true".to_owned());
    }
    lines.join("\n") + "\n"
}

fn render_bring_plan_json(parsed: &ParsedBringArgs) -> String {
    let opts = &parsed.opts;
    let mut fields = vec![
        format!("\"oracle\":{}", json_string(&parsed.oracle)),
        format!("\"split\":{}", opts.split),
    ];
    push_json_opt(&mut fields, "engine", opts.engine.as_deref());
    if opts.pick {
        fields.push("\"pick\":true".to_owned());
    }
    push_json_opt(&mut fields, "session", opts.session.as_deref());
    push_json_opt(&mut fields, "splitTarget", opts.split_target.as_deref());
    format!(
        "{{\"command\":\"bring\",\"opts\":{{{}}}}}\n",
        fields.join(",")
    )
}

fn push_json_opt(fields: &mut Vec<String>, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        fields.push(format!("{}:{}", json_string(key), json_string(value)));
    }
}

fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => {
                let _ = write!(out, "\\u{:04x}", ch as u32);
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

#[allow(dead_code)]
const fn _assert_options_shape(_: &BringAliasOptions) {}
