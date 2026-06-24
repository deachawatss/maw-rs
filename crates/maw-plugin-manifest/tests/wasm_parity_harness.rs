use std::fs::create_dir_all;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use maw_plugin_manifest::{
    hash_file, invoke_plugin, BunInvokeRuntime, ExtismWasmInvokeRuntime, InvokeContext,
    InvokeResult, InvokeSource, LoadedPlugin, LoadedPluginKind, MawWasmHost, PluginManifest,
};
use serde_json::Value;

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn golden_parity_trivial_bun_and_wasm_outputs_match_in_isolated_maw_home() {
    run_parity_case(ParityCase {
        plugin: "trivial",
        manifest_name: "trivial-parity",
        args: &["alpha", "beta"],
        expected_host_calls: None,
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
        real_maw_js_entry: RealMawJsEntry::CrossTeamQueueHandle,
    });
}

#[test]
#[ignore = "regenerates committed maw-js parity goldens; requires MAW_JS_REF_DIR"]
fn generate_wasm_parity_goldens_from_real_maw_js() {
    for case in parity_cases() {
        generate_golden(case);
    }
}

fn parity_cases() -> Vec<ParityCase<'static>> {
    let mut cases = vec![ParityCase {
        plugin: "trivial",
        manifest_name: "trivial-parity",
        args: &["alpha", "beta"],
        expected_host_calls: None,
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
        real_maw_js_entry: RealMawJsEntry::CrossTeamQueueHandle,
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
struct ParityCase<'a> {
    plugin: &'a str,
    manifest_name: &'a str,
    args: &'a [&'a str],
    expected_host_calls: Option<usize>,
    real_maw_js_entry: RealMawJsEntry,
}

#[derive(Clone, Copy)]
enum RealMawJsEntry {
    DefaultHandler(&'static str),
    CrossTeamQueueHandle,
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
    if let Some(expected) = case.expected_host_calls {
        let audit = host_audit.audit_json_lines();
        let actual = audit.lines().filter(|line| !line.trim().is_empty()).count();
        assert_eq!(
            actual, expected,
            "host-call audit mismatch for {} {:?}: {audit}",
            case.plugin, case.args
        );
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

fn seeded_host(fixture: &Path, plugin: &LoadedPlugin) -> MawWasmHost {
    let host_state: Value = serde_json::from_str(
        &std::fs::read_to_string(fixture.join("host-state.json")).expect("host-state"),
    )
    .expect("host-state json");
    host_state["calls"].as_array().expect("calls").iter().fold(
        MawWasmHost::new(plugin),
        |host, call| {
            host.with_fake_response(
                call["name"].as_str().expect("fake name"),
                call["input"].as_str().expect("fake input"),
                call["output"].as_str().expect("fake output"),
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

fn load_wasm_fixture(dir: &Path, manifest_name: &str) -> LoadedPlugin {
    LoadedPlugin {
        manifest: manifest(manifest_name),
        dir: dir.to_path_buf(),
        wasm_path: dir.join("plugin.wasm"),
        entry_path: None,
        wasm_export: "handle".to_owned(),
        kind: LoadedPluginKind::Wasm,
        disabled: false,
    }
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
