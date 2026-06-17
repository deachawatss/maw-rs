// Minimal side-by-side maw-rs CLI dry-run surfaces.
//
// This crate intentionally starts with plan-only output so command parity can
// be tested against maw-js parser contracts before host IO is wired.

use maw_auth::{
    apply_consent_expiry, approve_consent_plan, build_from_sign_payload,
    build_legacy_from_sign_payload, consent_request_id_from_bytes, generate_pair_code_from_bytes,
    hash_body, hash_consent_pin, is_loopback, is_valid_pair_code_shape, normalize_pair_code,
    pair_api_accept_plan, pair_api_auto_plan, pair_api_generate_plan, pair_api_probe_plan,
    pair_api_status_plan, pretty_pair_code, redact_pair_code, reject_consent_plan,
    request_consent_plan, resolve_from_address, sign, sign_auto_pair_proof, sign_headers_at,
    sign_headers_v3_at, sign_hmac_sig, sign_request_v3, trust_key, verify, verify_auto_pair_proof,
    verify_consent_pin, verify_hmac_sig, verify_request, ApprovedBy, AutoPairAddOutcome,
    AutoPairIdentity, AutoPairInput, ConsentAction, ConsentApprovalResult, ConsentRequestArgs,
    ConsentRequestResult, ConsentStatus, ConsentStore, FromAddressConfig, FromVerifyDecision,
    Headers, LookupResult, PairAcceptInput, PairApiAcceptResult, PairApiAutoResult, PairApiConfig,
    PairApiGenerateResult, PairApiProbeResult, PairApiStatusResult, PairCodeStore, PairEntry,
    PeerPendingRequest, PeerPostResult, PendingRequest, RecentHelloStore, TrustEntry,
    VerifyRequestArgs, DEFAULT_ORACLE, PAIR_CODE_ALPHABET, WINDOW_SEC,
};
use maw_auto_wake::{should_auto_wake, AutoWakeManifest, AutoWakeOptions, AutoWakeSite};
use maw_bind::{resolve_bind_host, BindConfig, BindHostResult};
use maw_bring::{parse_bring_args, ParsedBringArgs};
use maw_calver::{compute_version, Channel, ComputeArgs, DateParts};
use maw_feed::{active_oracles_at, describe_activity, parse_line, FeedEvent};
use maw_fuzzy::{distance as fuzzy_distance, fuzzy_match};
use maw_hub::{
    load_workspace_configs, validate_workspace_config, WorkspaceConfig, WorkspaceConfigValidation,
    HEARTBEAT_MS, RECONNECT_BASE_MS, RECONNECT_MAX_MS,
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
    MvpWasmInvokeRuntime, PluginInvokeRuntime, PluginManifest,
};
use maw_plugin_scaffold::{
    build_manifest_json, validate_plugin_name, PluginLanguage as ScaffoldLanguage,
};
use maw_policy::{default_active_group, weight_to_tier, DEFAULT_TIER, KNOWN_TIERS};
use maw_routing::{
    apply_sync_diff, compute_sync_diff, hosted_agents, resolve_target as resolve_route_target,
    MawConfig as RouteConfig, NamedPeer as RouteNamedPeer, PeerIdentity as SyncPeerIdentity,
    ResolveResult as RouteResult, Session as RouteSession, SyncApplyOptions, SyncApplyResult,
    SyncDiff, Window as RouteWindow,
};
use maw_split::{decide_split_policy, SplitPolicyDecision, SplitPolicyInput};
use maw_tmux::{
    decide_tmux_attach_action, mark_peer_targets_live, resolve_tmux_live_state,
    resolve_tmux_attach_session, tmux_attach_spawn_command, DiscoverLivePane, PeerTargetWithLive,
    TmuxAttachAction, TmuxAttachSessionResolution, TmuxClient, TmuxLiveStateResult, TmuxPane,
};
use maw_transport::{
    classify_error, classify_symmetric_federation_status, FederationPeerStatus, FederationPeerView,
    FederationStatus, PairStatus, PeerFederationStatus, PeerFederationStatusResult,
    SymmetricFederationStatus, Transport, TransportFailureReason, TransportResult, TransportRouter,
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
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
struct FederationSyncFlags {
    dry_run: bool,
    check: bool,
    force: bool,
    prune: bool,
}

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
        "attach" | "a" => run_attach_plan(&argv[1..]),
        "bring" | "b" => run_bring_plan(&argv[1..]),
        "ls" => run_ls_plan(&argv[1..]),
        "feed" => run_feed_plan(&argv[1..]),
        "fuzzy" => run_fuzzy_plan(&argv[1..]),
        "resolve" => run_resolve_plan(&argv[1..]),
        "identity" => run_identity_plan(&argv[1..]),
        "normalize" => run_normalize_plan(&argv[1..]),
        "calver" => run_calver_plan(&argv[1..]),
        "worktree-window" => run_worktree_window_plan(&argv[1..]),
        "route" => run_route_plan(&argv[1..]),
        "discover" => run_discover_plan(&argv[1..]),
        "federation-identity" => run_federation_identity_plan(&argv[1..]),
        "federation-health" => run_federation_health_plan(&argv[1..]),
        "federation-sync" => run_federation_sync_plan(&argv[1..]),
        "auto-pair-proof" => run_auto_pair_proof_plan(&argv[1..]),
        "consent-constants" => run_consent_constants_plan(&argv[1..]),
        "consent-pin" => run_consent_pin_plan(&argv[1..]),
        "consent-request" => run_consent_request_plan(&argv[1..]),
        "consent-approval" => run_consent_approval_plan(&argv[1..]),
        "consent-store" => run_consent_store_plan(&argv[1..]),
        "consent-expiry" => run_consent_expiry_plan(&argv[1..]),
        "consent-cleanup" => run_consent_cleanup_plan(&argv[1..]),
        "consent-trust-revoke" => run_consent_trust_revoke_plan(&argv[1..]),
        "consent-trust-check" => run_consent_trust_check_plan(&argv[1..]),
        "consent-pending-read" => run_consent_pending_read_plan(&argv[1..]),
        "consent-pending-status" => run_consent_pending_status_plan(&argv[1..]),
        "recent-hello" => run_recent_hello_plan(&argv[1..]),
        "pair-code" => run_pair_code_plan(&argv[1..]),
        "pair-code-store" => run_pair_code_store_plan(&argv[1..]),
        "pair-api" => run_pair_api_plan(&argv[1..]),
        "pair-api-auto" => run_pair_api_auto_plan(&argv[1..]),
        "peer-sources" => run_peer_sources_plan(&argv[1..]),
        "peer-probe" => run_peer_probe_plan(&argv[1..]),
        "policy" | "plugin-policy" => run_policy_plan(&argv[1..]),
        "split-policy" => run_split_policy_plan(&argv[1..]),
        "transport" => run_transport_plan(&argv[1..]),
        _ => dispatch_cli_plugin(argv).unwrap_or_else(|| CliOutput {
            code: 2,
            stdout: String::new(),
            stderr: format!("unknown command: {command}\n{}", usage_text()),
        }),
    }
}

fn dispatch_cli_plugin(argv: &[String]) -> Option<CliOutput> {
    let report = discover_packages(&DiscoverPackagesOptions::default());
    let (plugin, matched_args) = report
        .plugins
        .iter()
        .filter(|plugin| !plugin.disabled)
        .find_map(|plugin| plugin_cli_args(plugin, argv).map(|args| (plugin, args)))?;

    let ctx = InvokeContext {
        source: InvokeSource::Cli,
        args: matched_args.to_vec(),
    };
    let mut runtime = MvpWasmInvokeRuntime;
    Some(render_cli_plugin_result(invoke_plugin(plugin, &ctx, &mut runtime)))
}

fn plugin_cli_args<'a>(plugin: &LoadedPlugin, argv: &'a [String]) -> Option<&'a [String]> {
    let command = &plugin.manifest.cli.as_ref()?.command;
    let command_parts = command.split_whitespace().collect::<Vec<_>>();
    if command_parts.is_empty() || argv.len() < command_parts.len() {
        return None;
    }
    argv.iter()
        .map(String::as_str)
        .zip(&command_parts)
        .all(|(arg, command_part)| arg == *command_part)
        .then_some(&argv[command_parts.len()..])
}

fn render_cli_plugin_result(result: InvokeResult) -> CliOutput {
    if result.ok {
        return CliOutput {
            code: 0,
            stdout: result.output.map_or_else(String::new, with_trailing_newline),
            stderr: String::new(),
        };
    }

    CliOutput {
        code: 1,
        stdout: String::new(),
        stderr: with_trailing_newline(
            result
                .error
                .unwrap_or_else(|| "plugin invocation failed".to_owned()),
        ),
    }
}

fn with_trailing_newline(mut value: String) -> String {
    if !value.ends_with('\n') {
        value.push('\n');
    }
    value
}


#[allow(clippy::too_many_lines)]
fn run_auto_wake_plan(argv: &[String]) -> CliOutput {
    if matches!(argv.first().map(String::as_str), Some("constants")) {
        return run_auto_wake_constants_plan(&argv[1..]);
    }

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

fn run_auto_wake_constants_plan(argv: &[String]) -> CliOutput {
    let mut plan_json = false;
    for arg in argv {
        match arg.as_str() {
            "--plan-json" => plan_json = true,
            arg => {
                return auto_wake_constants_usage_error(&format!(
                    "auto-wake constants: unknown arg {arg}"
                ))
            }
        }
    }

    CliOutput {
        code: 0,
        stdout: if plan_json {
            render_auto_wake_constants_json()
        } else {
            "auto-wake constants sites=view,hey,api-send,api-wake,peek,bud,wake-cmd flags=fleet-known,unknown-fleet,live,not-live,wake,no-wake,canonical-target\n".to_owned()
        },
        stderr: String::new(),
    }
}

fn render_auto_wake_constants_json() -> String {
    r#"{"command":"auto-wake","action":"constants","sites":["view","hey","api-send","api-wake","peek","bud","wake-cmd"],"fleetFlags":["fleet-known","unknown-fleet"],"livenessFlags":["live","not-live"],"overrideFlags":["wake","no-wake"],"targetFlags":["canonical-target"],"manifestFields":["manifest-source","manifest-live"],"manifestLiveValues":["true","false"]}
"#
    .to_owned()
}

fn auto_wake_constants_usage_error(message: &str) -> CliOutput {
    CliOutput {
        code: 2,
        stdout: String::new(),
        stderr: format!("{message}\n{}\n", auto_wake_constants_usage()),
    }
}

fn auto_wake_constants_usage() -> &'static str {
    "usage: maw-rs auto-wake constants [--plan-json]"
}

fn auto_wake_usage_error(message: &str) -> CliOutput {
    CliOutput {
        code: 2,
        stdout: String::new(),
        stderr: format!("{message}\n{}\n", auto_wake_usage()),
    }
}

fn auto_wake_usage() -> &'static str {
    "usage: maw-rs auto-wake <target> --site <view|hey|api-send|api-wake|peek|bud|wake-cmd> [--fleet-known|--unknown-fleet] [--live|--not-live] [--wake] [--no-wake] [--canonical-target] [--manifest-source <source>]... [--manifest-live <true|false>] [--plan-json]
       maw-rs auto-wake constants [--plan-json]"
}

#[allow(clippy::too_many_lines)]
fn run_auth_plan(argv: &[String]) -> CliOutput {
    let action = match parse_auth_plan_args(argv) {
        Ok(action) => action,
        Err(message) => return auth_usage_error(&message),
    };
    match action {
        AuthPlanAction::SignV1 {
            plan_json,
            token,
            method,
            path,
            timestamp,
            body_hash,
        } => run_auth_sign_v1(plan_json, &token, &method, &path, timestamp, &body_hash),
        AuthPlanAction::SignHeaders {
            plan_json,
            token,
            method,
            path,
            timestamp,
            body,
        } => run_auth_sign_headers(
            plan_json,
            &token,
            &method,
            &path,
            timestamp,
            body.as_deref(),
        ),
        AuthPlanAction::VerifyV1 {
            plan_json,
            token,
            method,
            path,
            signed_at,
            now,
            signature,
            body_hash,
        } => run_auth_verify_v1(
            plan_json, &token, &method, &path, signed_at, now, &signature, &body_hash,
        ),
        AuthPlanAction::VerifyLegacyFrom {
            plan_json,
            cached_pubkey,
            from,
            signed_at,
            signature,
            method,
            path,
            now,
            body,
        } => run_auth_verify_legacy_from(
            plan_json,
            cached_pubkey.as_deref(),
            &from,
            &signed_at,
            &signature,
            &method,
            &path,
            now,
            body,
        ),
        AuthPlanAction::VerifyV3From {
            plan_json,
            cached_pubkey,
            from,
            timestamp,
            signature_v3,
            method,
            path,
            now,
            body,
        } => run_auth_verify_v3_from(
            plan_json,
            cached_pubkey.as_deref(),
            &from,
            timestamp,
            &signature_v3,
            &method,
            &path,
            now,
            body,
        ),
        AuthPlanAction::FromSignPayload {
            plan_json,
            legacy,
            from,
            timestamp,
            signed_at,
            method,
            path,
            body_hash,
        } => run_auth_from_sign_payload(
            plan_json,
            legacy,
            &from,
            timestamp,
            signed_at.as_deref(),
            &method,
            &path,
            &body_hash,
        ),
        AuthPlanAction::HmacVerify {
            plan_json,
            secret,
            payload,
            signature,
        } => run_auth_hmac_verify(plan_json, &secret, &payload, &signature),
        AuthPlanAction::HmacSign {
            plan_json,
            secret,
            payload,
        } => run_auth_hmac_sign(plan_json, &secret, &payload),
        AuthPlanAction::Constants { plan_json } => run_auth_constants(plan_json),
        AuthPlanAction::SignV3 {
            plan_json,
            peer_key,
            from_address,
            method,
            path,
            timestamp,
            body,
        } => run_auth_sign_v3(
            plan_json,
            &peer_key,
            &from_address,
            &method,
            &path,
            timestamp,
            body.as_deref(),
        ),
        AuthPlanAction::Loopback { plan_json, address } => run_auth_loopback(plan_json, &address),
        AuthPlanAction::FromAddress {
            plan_json,
            oracle,
            node,
        } => run_auth_from_address(plan_json, oracle.as_deref(), &node),
        AuthPlanAction::HashBody { plan_json, body } => {
            run_auth_hash_body(plan_json, body.as_deref())
        }
        AuthPlanAction::VerifyRequest {
            plan_json,
            method,
            path,
            timestamp,
            body,
            cached_pubkey,
            headers,
        } => run_auth_verify_request(
            plan_json,
            method,
            path,
            timestamp,
            body,
            cached_pubkey,
            headers,
        ),
    }
}
