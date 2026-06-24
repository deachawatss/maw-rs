use std::fs::create_dir_all;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use maw_plugin_manifest::{
    hash_file, invoke_plugin, load_manifest_from_dir, BunInvokeRuntime, ExtismWasmInvokeRuntime,
    InvokeContext, InvokeResult, InvokeSource, LoadedPlugin, LoadedPluginKind, MawWasmHost,
    PluginManifest,
};
use serde_json::Value;

static ENV_LOCK: Mutex<()> = Mutex::new(());

const PROFILE_CURRENT_TRANSCRIPT: &[ExpectedHostCall] = &[ExpectedHostCall::new(
    "maw.fs.read",
    "fs:read:config",
    "/config/profile-active",
)];
const PROFILE_LIST_TRANSCRIPT: &[ExpectedHostCall] = &[
    ExpectedHostCall::new("maw.fs.read", "fs:read:config", "/config/profile-active"),
    ExpectedHostCall::new("maw.fs.read", "fs:read:config", "/config/profiles/all.json"),
    ExpectedHostCall::new(
        "maw.fs.read",
        "fs:read:config",
        "/config/profiles/minimal.json",
    ),
];
const PROFILE_SHOW_MINIMAL_TRANSCRIPT: &[ExpectedHostCall] = &[ExpectedHostCall::new(
    "maw.fs.read",
    "fs:read:config",
    "/config/profiles/minimal.json",
)];
const PROFILE_USE_ALL_TRANSCRIPT: &[ExpectedHostCall] = &[
    ExpectedHostCall::new("maw.fs.read", "fs:read:config", "/config/profiles/all.json"),
    ExpectedHostCall::new("maw.fs.write", "fs:write:config", "/config/profile-active"),
];

const CONFIG_SET_NODE_TRANSCRIPT: &[ExpectedHostCall] = &[ExpectedHostCall::new(
    "maw.config.set",
    "sdk:config:write",
    "config:node",
)];
const CONFIG_SET_PORT_JSON_TRANSCRIPT: &[ExpectedHostCall] = &[ExpectedHostCall::new(
    "maw.config.set",
    "sdk:config:write",
    "config:port",
)];
const CONSENT_LIST_TRANSCRIPT: &[ExpectedHostCall] = &[ExpectedHostCall::new(
    "maw.consent.read",
    "sdk:consent:read",
    "consent:pending",
)];
const CONSENT_LIST_TRUST_TRANSCRIPT: &[ExpectedHostCall] = &[ExpectedHostCall::new(
    "maw.consent.read",
    "sdk:consent:read",
    "consent:trust",
)];

const PEEK_TRANSCRIPT: &[ExpectedHostCall] = &[
    ExpectedHostCall::new("maw.tmux.list_sessions", "tmux:read", "tmux://sessions"),
    ExpectedHostCall::new("maw.tmux.capture", "tmux:read", "wasm-parity-peek:0"),
];
const WORKSPACE_LS_TRANSCRIPT: &[ExpectedHostCall] = &[
    ExpectedHostCall::new("maw.fs.list", "fs:read:data", "/data/workspaces"),
    ExpectedHostCall::new("maw.fs.read", "fs:read:data", "/data/workspaces/alpha.json"),
    ExpectedHostCall::new("maw.fs.read", "fs:read:data", "/data/workspaces/beta.json"),
];
const SERVE_PEER_STARTUP_WARNINGS_TRANSCRIPT: &[ExpectedHostCall] = &[
    ExpectedHostCall::new("maw.fs.read", "fs:read:config", "/config/maw.config.json"),
    ExpectedHostCall::new("maw.fs.read", "fs:read:state", "/state/peers.json"),
];
const SEND_TRANSCRIPT: &[ExpectedHostCall] = &[ExpectedHostCall::new(
    "maw.tmux.send_keys",
    "tmux:send",
    "mawjs:codex-5.pane",
)];

#[test]
fn golden_parity_trivial_bun_and_wasm_outputs_match_in_isolated_maw_home() {
    run_parity_case(ParityCase {
        plugin: "trivial",
        manifest_name: "trivial-parity",
        args: &["alpha", "beta"],
        expected_host_calls: None,
        expected_host_transcript: None,
        real_maw_js_entry: RealMawJsEntry::DefaultHandler(
            "examples/wasm-parity/trivial/bun/index.ts",
        ),
    });
}

#[test]
fn golden_parity_shellenv_bun_and_wasm_outputs_match_in_isolated_maw_home() {
    for args in [&["zsh"][..], &["bash"][..], &["fish"][..], &[][..]] {
        run_parity_case(ParityCase {
            plugin: "shellenv",
            manifest_name: "shellenv-parity",
            args,
            expected_host_calls: Some(0),
            expected_host_transcript: None,
            real_maw_js_entry: RealMawJsEntry::DefaultHandler(
                "src/vendor/mpr-plugins/shellenv/src/index.ts",
            ),
        });
    }
}

#[test]
fn golden_parity_learn_bun_and_wasm_outputs_match_in_isolated_maw_home() {
    for args in [
        &["Soul-Brews-Studio/maw-js"][..],
        &["Soul-Brews-Studio/maw-js", "--fast"][..],
        &["Soul-Brews-Studio/maw-js", "--deep"][..],
        &["repo", "--fast", "--deep"][..],
        &["repo", "--turbo"][..],
        &[][..],
    ] {
        run_parity_case(ParityCase {
            plugin: "learn",
            manifest_name: "learn-parity",
            args,
            expected_host_calls: Some(0),
            expected_host_transcript: None,
            real_maw_js_entry: RealMawJsEntry::DefaultHandler(
                "src/vendor/mpr-plugins/learn/index.ts",
            ),
        });
    }
}

#[test]
fn golden_parity_cross_team_queue_bun_and_wasm_outputs_match_in_isolated_maw_home() {
    run_parity_case(ParityCase {
        plugin: "cross-team-queue",
        manifest_name: "cross-team-queue-parity",
        args: &[],
        expected_host_calls: Some(0),
        expected_host_transcript: None,
        real_maw_js_entry: RealMawJsEntry::CrossTeamQueueHandle,
    });
}

#[test]
fn golden_parity_project_bun_and_wasm_outputs_match_in_isolated_maw_home() {
    for args in [
        &[][..],
        &["learn", "https://github.com/Soul-Brews-Studio/maw-js"][..],
        &["incubate", "https://github.com/Soul-Brews-Studio/maw-rs"][..],
        &["find", "oracle"][..],
        &["search", "ψ"][..],
        &["list"][..],
        &["bogus"][..],
        &["learn"][..],
    ] {
        run_parity_case(ParityCase {
            plugin: "project",
            manifest_name: "project-parity",
            args,
            expected_host_calls: Some(0),
            expected_host_transcript: None,
            real_maw_js_entry: RealMawJsEntry::DefaultHandler(
                "src/vendor/mpr-plugins/project/index.ts",
            ),
        });
    }
}

#[test]
fn golden_parity_triggers_bun_and_wasm_outputs_match_in_isolated_maw_home() {
    run_parity_case(ParityCase {
        plugin: "triggers",
        manifest_name: "triggers-parity",
        args: &[],
        expected_host_calls: Some(0),
        expected_host_transcript: None,
        real_maw_js_entry: RealMawJsEntry::DefaultHandler(
            "src/vendor/mpr-plugins/triggers/index.ts",
        ),
    });
}

#[test]
fn golden_parity_profile_bun_and_wasm_outputs_match_seeded_host() {
    for (args, expected_host_transcript) in [
        (&["current"][..], PROFILE_CURRENT_TRANSCRIPT),
        (&["list"][..], PROFILE_LIST_TRANSCRIPT),
        (&["show", "minimal"][..], PROFILE_SHOW_MINIMAL_TRANSCRIPT),
        (&["use", "all"][..], PROFILE_USE_ALL_TRANSCRIPT),
    ] {
        run_parity_case(ParityCase {
            plugin: "profile",
            manifest_name: "profile-parity",
            args,
            expected_host_calls: Some(expected_host_transcript.len()),
            expected_host_transcript: Some(expected_host_transcript),
            real_maw_js_entry: RealMawJsEntry::DefaultHandler(
                "src/vendor/mpr-plugins/profile/index.ts",
            ),
        });
    }
}

#[test]
fn golden_parity_config_set_bun_and_wasm_outputs_match_seeded_host() {
    for (args, expected_host_transcript) in [
        (
            &["set", "node", "nova-node"][..],
            CONFIG_SET_NODE_TRANSCRIPT,
        ),
        (
            &["set", "port", "4567", "--json"][..],
            CONFIG_SET_PORT_JSON_TRANSCRIPT,
        ),
    ] {
        run_parity_case(ParityCase {
            plugin: "config",
            manifest_name: "config-parity",
            args,
            expected_host_calls: Some(expected_host_transcript.len()),
            expected_host_transcript: Some(expected_host_transcript),
            real_maw_js_entry: RealMawJsEntry::DefaultHandler(
                "src/commands/plugins/config/index.ts",
            ),
        });
    }
}

#[test]
fn golden_parity_consent_read_only_bun_and_wasm_outputs_match_seeded_host() {
    for (args, expected_host_transcript) in [
        (&[][..], CONSENT_LIST_TRANSCRIPT),
        (&["list"][..], CONSENT_LIST_TRANSCRIPT),
        (&["list-trust"][..], CONSENT_LIST_TRUST_TRANSCRIPT),
    ] {
        run_parity_case(ParityCase {
            plugin: "consent",
            manifest_name: "consent-parity",
            args,
            expected_host_calls: Some(expected_host_transcript.len()),
            expected_host_transcript: Some(expected_host_transcript),
            real_maw_js_entry: RealMawJsEntry::DefaultHandler(
                "src/vendor/mpr-plugins/consent/index.ts",
            ),
        });
    }
}

#[test]
fn config_wasm_denies_secret_like_set_without_host_call() {
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wasm-parity/config");
    let wasm_plugin = load_wasm_fixture(&fixture, "config-parity");
    assert_eq!(
        wasm_plugin.manifest.capabilities.as_deref(),
        Some(&["sdk:config:write".to_owned()][..]),
        "config-write must declare only sdk:config:write"
    );
    let host = seeded_host(&fixture, &wasm_plugin);
    let host_audit = host.clone();
    let mut wasm_runtime =
        ExtismWasmInvokeRuntime::default().with_host(wasm_plugin.manifest.name.clone(), host);
    let ctx = InvokeContext {
        source: InvokeSource::Cli,
        args: ["set", "federationToken", "secret-value"]
            .iter()
            .map(|arg| (*arg).to_owned())
            .collect(),
    };

    let result = invoke_plugin(&wasm_plugin, &ctx, &mut wasm_runtime);

    assert!(!result.ok, "secret-like config write must fail");
    assert_eq!(
        result.error.as_deref(),
        Some("maw config set: secret-like keys are host-gated and cannot be written from WASM")
    );
    assert_eq!(
        host_audit.audit_json_lines(),
        "",
        "secret write reached host"
    );
}

#[test]
fn golden_parity_peek_bun_and_wasm_outputs_match_seeded_host() {
    run_parity_case(ParityCase {
        plugin: "peek",
        manifest_name: "peek-parity",
        args: &["wasm-parity-peek:0"],
        expected_host_calls: Some(PEEK_TRANSCRIPT.len()),
        expected_host_transcript: Some(PEEK_TRANSCRIPT),
        real_maw_js_entry: RealMawJsEntry::DefaultHandler("src/vendor/mpr-plugins/peek/index.ts"),
    });
}

#[test]
fn golden_parity_workspace_ls_bun_and_wasm_outputs_match_seeded_host() {
    for args in [&[][..], &["ls"][..], &["list"][..]] {
        run_parity_case(ParityCase {
            plugin: "workspace",
            manifest_name: "workspace-parity",
            args,
            expected_host_calls: Some(WORKSPACE_LS_TRANSCRIPT.len()),
            expected_host_transcript: Some(WORKSPACE_LS_TRANSCRIPT),
            real_maw_js_entry: RealMawJsEntry::DefaultHandler(
                "src/vendor/mpr-plugins/workspace/index.ts",
            ),
        });
    }
}

#[test]
fn golden_parity_serve_peer_startup_warnings_bun_and_wasm_outputs_match_seeded_host() {
    run_parity_case(ParityCase {
        plugin: "serve-peer-startup-warnings",
        manifest_name: "serve-peer-startup-warnings-parity",
        args: &[],
        expected_host_calls: Some(SERVE_PEER_STARTUP_WARNINGS_TRANSCRIPT.len()),
        expected_host_transcript: Some(SERVE_PEER_STARTUP_WARNINGS_TRANSCRIPT),
        real_maw_js_entry: RealMawJsEntry::DefaultHandler(
            "src/vendor-plugins/serve-peer-startup-warnings/index.ts",
        ),
    });
}

#[test]
fn golden_parity_send_bun_and_wasm_outputs_match_seeded_host() {
    run_parity_case(ParityCase {
        plugin: "send",
        manifest_name: "send-parity",
        args: &["mawjs:codex-5.pane", "hello", "world"],
        expected_host_calls: Some(SEND_TRANSCRIPT.len()),
        expected_host_transcript: Some(SEND_TRANSCRIPT),
        real_maw_js_entry: RealMawJsEntry::SendReadOnlyWrapper,
    });
}

#[test]
fn send_wasm_declares_plain_send_only_for_non_destructive_fixture() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wasm-parity/send");
    let wasm_plugin = load_wasm_fixture(&fixture, "send-parity");
    assert_eq!(
        wasm_plugin.manifest.capabilities.as_deref(),
        Some(&["tmux:send".to_owned()][..]),
        "non-destructive send fixture must not over-grant tmux:send:force"
    );
}

#[test]
#[ignore = "regenerates committed maw-js parity goldens; requires MAW_JS_REF_DIR"]
fn generate_wasm_parity_goldens_from_real_maw_js() {
    for case in parity_cases() {
        generate_golden(case);
    }
}

#[allow(clippy::too_many_lines)]
fn parity_cases() -> Vec<ParityCase<'static>> {
    let mut cases = vec![ParityCase {
        plugin: "trivial",
        manifest_name: "trivial-parity",
        args: &["alpha", "beta"],
        expected_host_calls: None,
        expected_host_transcript: None,
        real_maw_js_entry: RealMawJsEntry::DefaultHandler(
            "examples/wasm-parity/trivial/bun/index.ts",
        ),
    }];

    for args in [&["zsh"][..], &["bash"][..], &["fish"][..], &[][..]] {
        cases.push(ParityCase {
            plugin: "shellenv",
            manifest_name: "shellenv-parity",
            args,
            expected_host_calls: Some(0),
            expected_host_transcript: None,
            real_maw_js_entry: RealMawJsEntry::DefaultHandler(
                "src/vendor/mpr-plugins/shellenv/src/index.ts",
            ),
        });
    }

    for args in [
        &["Soul-Brews-Studio/maw-js"][..],
        &["Soul-Brews-Studio/maw-js", "--fast"][..],
        &["Soul-Brews-Studio/maw-js", "--deep"][..],
        &["repo", "--fast", "--deep"][..],
        &["repo", "--turbo"][..],
        &[][..],
    ] {
        cases.push(ParityCase {
            plugin: "learn",
            manifest_name: "learn-parity",
            args,
            expected_host_calls: Some(0),
            expected_host_transcript: None,
            real_maw_js_entry: RealMawJsEntry::DefaultHandler(
                "src/vendor/mpr-plugins/learn/index.ts",
            ),
        });
    }

    cases.push(ParityCase {
        plugin: "cross-team-queue",
        manifest_name: "cross-team-queue-parity",
        args: &[],
        expected_host_calls: Some(0),
        expected_host_transcript: None,
        real_maw_js_entry: RealMawJsEntry::CrossTeamQueueHandle,
    });

    for args in [
        &[][..],
        &["learn", "https://github.com/Soul-Brews-Studio/maw-js"][..],
        &["incubate", "https://github.com/Soul-Brews-Studio/maw-rs"][..],
        &["find", "oracle"][..],
        &["search", "ψ"][..],
        &["list"][..],
        &["bogus"][..],
        &["learn"][..],
    ] {
        cases.push(ParityCase {
            plugin: "project",
            manifest_name: "project-parity",
            args,
            expected_host_calls: Some(0),
            expected_host_transcript: None,
            real_maw_js_entry: RealMawJsEntry::DefaultHandler(
                "src/vendor/mpr-plugins/project/index.ts",
            ),
        });
    }

    cases.push(ParityCase {
        plugin: "triggers",
        manifest_name: "triggers-parity",
        args: &[],
        expected_host_calls: Some(0),
        expected_host_transcript: None,
        real_maw_js_entry: RealMawJsEntry::DefaultHandler(
            "src/vendor/mpr-plugins/triggers/index.ts",
        ),
    });

    for (args, expected_host_transcript) in [
        (&["current"][..], PROFILE_CURRENT_TRANSCRIPT),
        (&["list"][..], PROFILE_LIST_TRANSCRIPT),
        (&["show", "minimal"][..], PROFILE_SHOW_MINIMAL_TRANSCRIPT),
        (&["use", "all"][..], PROFILE_USE_ALL_TRANSCRIPT),
    ] {
        cases.push(ParityCase {
            plugin: "profile",
            manifest_name: "profile-parity",
            args,
            expected_host_calls: Some(expected_host_transcript.len()),
            expected_host_transcript: Some(expected_host_transcript),
            real_maw_js_entry: RealMawJsEntry::DefaultHandler(
                "src/vendor/mpr-plugins/profile/index.ts",
            ),
        });
    }

    for (args, expected_host_transcript) in [
        (
            &["set", "node", "nova-node"][..],
            CONFIG_SET_NODE_TRANSCRIPT,
        ),
        (
            &["set", "port", "4567", "--json"][..],
            CONFIG_SET_PORT_JSON_TRANSCRIPT,
        ),
    ] {
        cases.push(ParityCase {
            plugin: "config",
            manifest_name: "config-parity",
            args,
            expected_host_calls: Some(expected_host_transcript.len()),
            expected_host_transcript: Some(expected_host_transcript),
            real_maw_js_entry: RealMawJsEntry::DefaultHandler(
                "src/commands/plugins/config/index.ts",
            ),
        });
    }

    for (args, expected_host_transcript) in [
        (&[][..], CONSENT_LIST_TRANSCRIPT),
        (&["list"][..], CONSENT_LIST_TRANSCRIPT),
        (&["list-trust"][..], CONSENT_LIST_TRUST_TRANSCRIPT),
    ] {
        cases.push(ParityCase {
            plugin: "consent",
            manifest_name: "consent-parity",
            args,
            expected_host_calls: Some(expected_host_transcript.len()),
            expected_host_transcript: Some(expected_host_transcript),
            real_maw_js_entry: RealMawJsEntry::DefaultHandler(
                "src/vendor/mpr-plugins/consent/index.ts",
            ),
        });
    }

    cases.push(ParityCase {
        plugin: "peek",
        manifest_name: "peek-parity",
        args: &["wasm-parity-peek:0"],
        expected_host_calls: Some(PEEK_TRANSCRIPT.len()),
        expected_host_transcript: Some(PEEK_TRANSCRIPT),
        real_maw_js_entry: RealMawJsEntry::DefaultHandler("src/vendor/mpr-plugins/peek/index.ts"),
    });

    for args in [&[][..], &["ls"][..], &["list"][..]] {
        cases.push(ParityCase {
            plugin: "workspace",
            manifest_name: "workspace-parity",
            args,
            expected_host_calls: Some(WORKSPACE_LS_TRANSCRIPT.len()),
            expected_host_transcript: Some(WORKSPACE_LS_TRANSCRIPT),
            real_maw_js_entry: RealMawJsEntry::DefaultHandler(
                "src/vendor/mpr-plugins/workspace/index.ts",
            ),
        });
    }

    cases.push(ParityCase {
        plugin: "serve-peer-startup-warnings",
        manifest_name: "serve-peer-startup-warnings-parity",
        args: &[],
        expected_host_calls: Some(SERVE_PEER_STARTUP_WARNINGS_TRANSCRIPT.len()),
        expected_host_transcript: Some(SERVE_PEER_STARTUP_WARNINGS_TRANSCRIPT),
        real_maw_js_entry: RealMawJsEntry::DefaultHandler(
            "src/vendor-plugins/serve-peer-startup-warnings/index.ts",
        ),
    });

    cases.push(ParityCase {
        plugin: "send",
        manifest_name: "send-parity",
        args: &["mawjs:codex-5.pane", "hello", "world"],
        expected_host_calls: Some(SEND_TRANSCRIPT.len()),
        expected_host_transcript: Some(SEND_TRANSCRIPT),
        real_maw_js_entry: RealMawJsEntry::SendReadOnlyWrapper,
    });

    cases
}

fn generate_golden(case: ParityCase<'_>) {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/wasm-parity")
        .join(case.plugin);
    assert_fixture_metadata(&fixture);

    let temp = temp_dir("wasm-parity-golden");
    let isolated_home = temp.join("home");
    create_dir_all(&isolated_home).expect("isolated MAW_HOME");
    seed_real_maw_home(&fixture, &isolated_home);
    let old_maw_home = std::env::var_os("MAW_HOME");
    let old_plugins_dir = std::env::var_os("MAW_PLUGINS_DIR");
    std::env::set_var("MAW_HOME", &isolated_home);
    std::env::remove_var("MAW_PLUGINS_DIR");

    let ctx = InvokeContext {
        source: InvokeSource::Cli,
        args: case.args.iter().map(|arg| (*arg).to_owned()).collect(),
    };

    let maw_js_ref = maw_js_ref_dir();
    let maw_js_provenance = maw_js_provenance(&maw_js_ref);
    let bun_entry = real_maw_js_entry_path(&temp, &maw_js_ref, case.real_maw_js_entry);
    let bun_plugin = make_bun_plugin(&bun_entry, case.manifest_name);
    let mut bun_runtime = BunInvokeRuntime::default();
    let bun = invoke_plugin(&bun_plugin, &ctx, &mut bun_runtime);

    restore_env("MAW_HOME", old_maw_home);
    restore_env("MAW_PLUGINS_DIR", old_plugins_dir);
    let _ = std::fs::remove_dir_all(temp);

    let golden = golden_path(&fixture, case.args);
    std::fs::write(
        &golden,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&capture(&bun)).expect("golden json")
        ),
    )
    .unwrap_or_else(|err| panic!("write {}: {err}", golden.display()));
    write_maw_js_provenance(&fixture, &maw_js_provenance);
}

#[derive(Clone)]
struct MawJsProvenance {
    version: Option<String>,
    commit: String,
}

fn maw_js_provenance(maw_js_ref: &Path) -> MawJsProvenance {
    assert!(
        maw_js_ref.exists(),
        "MAW_JS_REF_DIR must point at a maw-js checkout for golden refresh: {}",
        maw_js_ref.display()
    );
    let commit = command_stdout(
        Command::new("git")
            .arg("-C")
            .arg(maw_js_ref)
            .arg("rev-parse")
            .arg("HEAD"),
    );
    let package_json = maw_js_ref.join("package.json");
    let version = serde_json::from_str::<Value>(
        &std::fs::read_to_string(&package_json)
            .unwrap_or_else(|err| panic!("read {}: {err}", package_json.display())),
    )
    .unwrap_or_else(|err| panic!("parse {}: {err}", package_json.display()))
    .get("version")
    .and_then(Value::as_str)
    .map(str::to_owned);

    MawJsProvenance { version, commit }
}

fn write_maw_js_provenance(fixture: &Path, provenance: &MawJsProvenance) {
    let path = fixture.join("metadata.json");
    let mut metadata: Value = serde_json::from_str(
        &std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("read {}: {err}", path.display())),
    )
    .unwrap_or_else(|err| panic!("parse {}: {err}", path.display()));
    let obj = metadata.as_object_mut().expect("metadata object");
    if let Some(version) = &provenance.version {
        obj.insert("mawJsVersion".to_owned(), Value::String(version.clone()));
    }
    obj.insert(
        "mawJsCommit".to_owned(),
        Value::String(provenance.commit.clone()),
    );
    std::fs::write(
        &path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&metadata).expect("metadata json")
        ),
    )
    .unwrap_or_else(|err| panic!("write {}: {err}", path.display()));
}

fn command_stdout(command: &mut Command) -> String {
    let output = command.output().expect("run command");
    assert!(
        output.status.success(),
        "command failed status={:?} stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("utf8 stdout")
        .trim()
        .to_owned()
}

#[derive(Clone, Copy)]
struct ExpectedHostCall<'a> {
    host_fn: &'a str,
    capability: &'a str,
    resource: &'a str,
}

impl<'a> ExpectedHostCall<'a> {
    const fn new(host_fn: &'a str, capability: &'a str, resource: &'a str) -> Self {
        Self {
            host_fn,
            capability,
            resource,
        }
    }
}

#[derive(Clone, Copy)]
struct ParityCase<'a> {
    plugin: &'a str,
    manifest_name: &'a str,
    args: &'a [&'a str],
    expected_host_calls: Option<usize>,
    expected_host_transcript: Option<&'a [ExpectedHostCall<'a>]>,
    real_maw_js_entry: RealMawJsEntry,
}

#[derive(Clone, Copy)]
enum RealMawJsEntry {
    DefaultHandler(&'static str),
    CrossTeamQueueHandle,
    SendReadOnlyWrapper,
}

fn run_parity_case(case: ParityCase<'_>) {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/wasm-parity")
        .join(case.plugin);
    assert_fixture_metadata(&fixture);

    let temp = temp_dir("wasm-parity");
    let isolated_home = temp.join("home");
    create_dir_all(&isolated_home).expect("isolated MAW_HOME");
    seed_real_maw_home(&fixture, &isolated_home);
    let old_maw_home = std::env::var_os("MAW_HOME");
    let old_plugins_dir = std::env::var_os("MAW_PLUGINS_DIR");
    std::env::set_var("MAW_HOME", &isolated_home);
    std::env::remove_var("MAW_PLUGINS_DIR");

    let ctx = InvokeContext {
        source: InvokeSource::Cli,
        args: case.args.iter().map(|arg| (*arg).to_owned()).collect(),
    };

    let wasm_plugin = load_wasm_fixture(&fixture, case.manifest_name);
    let host = seeded_host(&fixture, &wasm_plugin);
    let host_audit = host.clone();
    let mut wasm_runtime =
        ExtismWasmInvokeRuntime::default().with_host(wasm_plugin.manifest.name.clone(), host);
    let wasm = invoke_plugin(&wasm_plugin, &ctx, &mut wasm_runtime);

    restore_env("MAW_HOME", old_maw_home);
    restore_env("MAW_PLUGINS_DIR", old_plugins_dir);
    let _ = std::fs::remove_dir_all(temp);

    assert_eq!(
        read_golden(&fixture, case.args),
        capture(&wasm),
        "plugin={} args={:?}",
        case.plugin,
        case.args
    );
    let audit = host_audit.audit_json_lines();
    if let Some(expected) = case.expected_host_calls {
        let actual = audit.lines().filter(|line| !line.trim().is_empty()).count();
        assert_eq!(
            actual, expected,
            "host-call audit mismatch for {} {:?}: {audit}",
            case.plugin, case.args
        );
    }
    if let Some(expected) = case.expected_host_transcript {
        assert_host_transcript(case.plugin, case.args, &audit, expected);
    }
}

fn assert_host_transcript(
    plugin: &str,
    args: &[&str],
    audit: &str,
    expected: &[ExpectedHostCall<'_>],
) {
    let actual = audit
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<Value>(line).expect("audit json"))
        .collect::<Vec<_>>();
    assert_eq!(
        actual.len(),
        expected.len(),
        "host transcript length for {plugin} {args:?}: {audit}"
    );
    for (event, expected) in actual.iter().zip(expected.iter()) {
        assert_eq!(
            event["host_fn"], expected.host_fn,
            "host_fn for {plugin} {args:?}"
        );
        assert_eq!(
            event["capability"], expected.capability,
            "capability for {plugin} {args:?}"
        );
        assert_eq!(
            event["resource"], expected.resource,
            "resource for {plugin} {args:?}"
        );
        assert_eq!(event["status"], "ok", "status for {plugin} {args:?}");
    }
}

fn assert_fixture_metadata(fixture: &Path) {
    let metadata: Value = serde_json::from_str(
        &std::fs::read_to_string(fixture.join("metadata.json")).expect("metadata"),
    )
    .expect("metadata json");
    assert_eq!(metadata["assemblyscript"], "0.27.31");
    assert_eq!(metadata["extismAsPdk"], "1.0.0");
    assert_eq!(
        hash_file(&fixture.join("plugin.wasm")).expect("wasm hash"),
        metadata["wasmSha256"].as_str().expect("sha")
    );
}

fn seed_real_maw_home(fixture: &Path, isolated_home: &Path) {
    let host_state_path = fixture.join("host-state.json");
    let host_state: Value = serde_json::from_str(
        &std::fs::read_to_string(&host_state_path)
            .unwrap_or_else(|err| panic!("read {}: {err}", host_state_path.display())),
    )
    .unwrap_or_else(|err| panic!("parse {}: {err}", host_state_path.display()));
    if let Some(files) = host_state.get("seedFiles").and_then(Value::as_object) {
        for (relative, content) in files {
            let path = isolated_home.join(relative);
            if let Some(parent) = path.parent() {
                create_dir_all(parent)
                    .unwrap_or_else(|err| panic!("create seed dir {}: {err}", parent.display()));
            }
            std::fs::write(&path, content.as_str().expect("seed file content"))
                .unwrap_or_else(|err| panic!("write seed file {}: {err}", path.display()));
        }
    }
}

fn seeded_host(fixture: &Path, plugin: &LoadedPlugin) -> MawWasmHost {
    let host_state: Value = serde_json::from_str(
        &std::fs::read_to_string(fixture.join("host-state.json")).expect("host-state"),
    )
    .expect("host-state json");
    host_state["calls"].as_array().expect("calls").iter().fold(
        MawWasmHost::new(plugin),
        |host, call| {
            host.with_audited_fake_response(
                call["name"].as_str().expect("fake name"),
                call["input"].as_str().expect("fake input"),
                call["output"].as_str().expect("fake output"),
                call.get("capability")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                call.get("resource")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                call.get("status")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            )
        },
    )
}

fn capture(result: &InvokeResult) -> Value {
    serde_json::json!({
        "stdout": result.output.as_deref().unwrap_or(""),
        "stderr": result.error.as_deref().unwrap_or(""),
        "result": { "ok": result.ok, "output": result.output, "error": result.error }
    })
}

fn read_golden(fixture: &Path, args: &[&str]) -> Value {
    let path = golden_path(fixture, args);
    serde_json::from_str(
        &std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("read golden {}: {err}", path.display())),
    )
    .unwrap_or_else(|err| panic!("parse golden {}: {err}", path.display()))
}

fn golden_path(fixture: &Path, args: &[&str]) -> PathBuf {
    fixture.join(format!("golden.{}.json", args_slug(args)))
}

fn args_slug(args: &[&str]) -> String {
    if args.is_empty() {
        return "no-args".to_owned();
    }
    args.iter()
        .map(|arg| {
            arg.chars()
                .map(|ch| match ch {
                    'a'..='z' | 'A'..='Z' | '0'..='9' => ch,
                    _ => '-',
                })
                .collect::<String>()
                .trim_matches('-')
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("--")
}

fn make_bun_plugin(entry_path: &Path, manifest_name: &str) -> LoadedPlugin {
    LoadedPlugin {
        manifest: manifest(manifest_name),
        dir: entry_path.parent().unwrap_or(entry_path).to_path_buf(),
        wasm_path: PathBuf::new(),
        entry_path: Some(entry_path.to_path_buf()),
        wasm_export: "handle".to_owned(),
        kind: LoadedPluginKind::Ts,
        disabled: false,
    }
}

fn maw_js_ref_dir() -> PathBuf {
    std::env::var_os("MAW_JS_REF_DIR").map_or_else(
        || PathBuf::from("/home/agent/github.com/Soul-Brews-Studio/maw-js"),
        PathBuf::from,
    )
}

fn real_maw_js_entry_path(temp: &Path, maw_js_ref: &Path, entry: RealMawJsEntry) -> PathBuf {
    match entry {
        RealMawJsEntry::DefaultHandler(relative) => {
            if relative.starts_with("examples/") {
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .ancestors()
                    .nth(2)
                    .expect("repo root")
                    .join(relative)
            } else {
                maw_js_ref.join(relative)
            }
        }
        RealMawJsEntry::CrossTeamQueueHandle => {
            write_cross_team_queue_real_wrapper(temp, maw_js_ref)
        }
        RealMawJsEntry::SendReadOnlyWrapper => write_send_real_wrapper(temp, maw_js_ref),
    }
}

fn write_cross_team_queue_real_wrapper(temp: &Path, maw_js_ref: &Path) -> PathBuf {
    let wrapper_dir = temp.join("real-maw-js-cross-team-queue");
    create_dir_all(&wrapper_dir).expect("cross-team-queue wrapper dir");
    let real_path = maw_js_ref
        .join("src/vendor/mpr-plugins/cross-team-queue/src/index.ts")
        .to_string_lossy()
        .to_string();
    let real = serde_json::to_string(&real_path).expect("real path json string");
    let wrapper = format!(
        "const real = await import({real});\nexport default async function handle(_ctx) {{\n  return {{ ok: true, output: JSON.stringify(await real.handle()) }};\n}}\n"
    );
    let path = wrapper_dir.join("index.ts");
    std::fs::write(&path, wrapper).expect("cross-team-queue wrapper");
    path
}

fn write_send_real_wrapper(temp: &Path, maw_js_ref: &Path) -> PathBuf {
    let wrapper_dir = temp.join("real-maw-js-send");
    create_dir_all(&wrapper_dir).expect("send wrapper dir");
    let sdk_path = maw_js_ref
        .join("src/sdk/index.ts")
        .to_string_lossy()
        .to_string();
    let send_path = maw_js_ref
        .join("src/vendor/mpr-plugins/send/index.ts")
        .to_string_lossy()
        .to_string();
    let sdk = serde_json::to_string(&sdk_path).expect("sdk path json string");
    let send = serde_json::to_string(&send_path).expect("send path json string");
    let wrapper = format!(
        r#"import {{ mock }} from "bun:test";
const realSdk = await import({sdk});
class MockTmux {{ async sendKeysLiteral(_target, _text) {{}} }}
mock.module("maw-js/sdk", () => ({{
  ...realSdk,
  loadConfig: () => ({{ node: {{ name: "local" }} }}),
  listSessions: async () => [{{ name: "mawjs", windows: [{{ name: "codex-5" }}] }}],
  resolveTarget: () => ({{ type: "local", target: "mawjs:codex-5" }}),
  resolveOraclePane: async (_target) => "mawjs:codex-5.pane",
  Tmux: MockTmux,
}}));
const real = await import({send});
export default real.default;
"#
    );
    let path = wrapper_dir.join("index.ts");
    std::fs::write(&path, wrapper).expect("send wrapper");
    path
}

fn load_wasm_fixture(dir: &Path, manifest_name: &str) -> LoadedPlugin {
    let mut plugin = load_manifest_from_dir(dir)
        .unwrap_or_else(|err| panic!("load fixture manifest {}: {err}", dir.display()))
        .unwrap_or_else(|| LoadedPlugin {
            manifest: manifest(manifest_name),
            dir: dir.to_path_buf(),
            wasm_path: dir.join("plugin.wasm"),
            entry_path: None,
            wasm_export: "handle".to_owned(),
            kind: LoadedPluginKind::Wasm,
            disabled: false,
        });
    manifest_name.clone_into(&mut plugin.manifest.name);
    plugin
}

fn manifest(name: &str) -> PluginManifest {
    PluginManifest {
        name: name.to_owned(),
        version: "1.0.0".to_owned(),
        weight: None,
        tier: None,
        wasm: None,
        entry: None,
        entry_export: Some("handle".to_owned()),
        sdk: "*".to_owned(),
        cli: None,
        api: None,
        description: None,
        author: None,
        hooks: None,
        cron: None,
        module: None,
        transport: None,
        engine: None,
        target: None,
        capability_namespaces: None,
        capabilities: Some(Vec::new()),
        capability_warnings: Vec::new(),
        dependencies: None,
        artifact: None,
    }
}

fn restore_env(key: &str, value: Option<std::ffi::OsString>) {
    if let Some(value) = value {
        std::env::set_var(key, value);
    } else {
        std::env::remove_var(key);
    }
}

fn temp_dir(prefix: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("maw-rs-{prefix}-{}-{stamp}", std::process::id()));
    create_dir_all(&path).expect("temp dir");
    path
}
