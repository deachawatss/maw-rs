use maw_plugin_manifest::{
    BunInvokeRuntime, InvokeContext, InvokeResult, InvokeSource, LoadedPlugin, LoadedPluginKind,
    PluginInvokeRuntime, PluginManifest,
};
use std::ffi::OsString;
use std::fs::{create_dir_all, read_to_string, remove_dir_all, write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct EnvRestore {
    path: Option<OsString>,
    capture_dir: Option<OsString>,
    mode: Option<OsString>,
}

impl EnvRestore {
    fn capture() -> Self {
        Self {
            path: std::env::var_os("PATH"),
            capture_dir: std::env::var_os("MAW_BUN_CAPTURE_DIR"),
            mode: std::env::var_os("MAW_BUN_MODE"),
        }
    }
}

impl Drop for EnvRestore {
    fn drop(&mut self) {
        restore_env("PATH", self.path.take());
        restore_env("MAW_BUN_CAPTURE_DIR", self.capture_dir.take());
        restore_env("MAW_BUN_MODE", self.mode.take());
    }
}

fn restore_env(key: &str, value: Option<OsString>) {
    if let Some(value) = value {
        std::env::set_var(key, value);
    } else {
        std::env::remove_var(key);
    }
}

fn temp_dir(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "maw-rs-bun-invoke-{label}-{}-{nonce}",
        std::process::id()
    ));
    create_dir_all(&dir).expect("create temp dir");
    dir
}

fn bun_available() -> bool {
    std::process::Command::new("bun")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn skip_without_bun(test_name: &str) -> bool {
    if bun_available() {
        return false;
    }
    eprintln!("skipping {test_name}: bun is not available on PATH");
    true
}

fn write_bun_shim(bin_dir: &Path) {
    let shim = bin_dir.join("bun");
    write(
        &shim,
        r#"#!/bin/sh
capture="$MAW_BUN_CAPTURE_DIR"
printf 'arg1=%s\narg3=%s\narg4=%s\narg5=%s\n' "$1" "$3" "$4" "$5" > "$capture/argv.txt"
/bin/cat > "$capture/stdin.json"
case "$MAW_BUN_MODE" in
  nonzero)
    printf 'boom\n' >&2
    exit 7
    ;;
  timeout)
    /bin/sleep 1
    printf '{"ok":true,"output":"late"}\n'
    ;;
  bad-json)
    printf 'not-json\n'
    ;;
  *)
    printf '{"ok":true,"output":"from bun"}\n'
    ;;
esac
"#,
    )
    .expect("write bun shim");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&shim)
            .expect("shim metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&shim, permissions).expect("chmod bun shim");
    }
}

fn setup_fake_bun(root: &Path) -> PathBuf {
    let bin_dir = root.join("bin");
    let capture_dir = root.join("capture");
    create_dir_all(&bin_dir).expect("bin dir");
    create_dir_all(&capture_dir).expect("capture dir");
    write_bun_shim(&bin_dir);
    std::env::set_var("PATH", &bin_dir);
    std::env::set_var("MAW_BUN_CAPTURE_DIR", &capture_dir);
    std::env::remove_var("MAW_BUN_MODE");
    capture_dir
}

fn make_ts_plugin(dir: &Path) -> LoadedPlugin {
    let entry_path = dir.join("index.ts");
    write(&entry_path, b"export default async () => ({ ok: true });\n").expect("entry");
    LoadedPlugin {
        manifest: PluginManifest {
            name: "demo-ts".to_owned(),
            version: "1.0.0".to_owned(),
            weight: None,
            tier: None,
            wasm: None,
            entry: Some("index.ts".to_owned()),
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
            capabilities: None,
            capability_warnings: Vec::new(),
            dependencies: None,
            artifact: None,
        },
        dir: dir.to_path_buf(),
        wasm_path: PathBuf::new(),
        entry_path: Some(entry_path),
        kind: LoadedPluginKind::Ts,
        disabled: false,
    }
}

fn make_real_handler_plugin(dir: &Path) -> LoadedPlugin {
    let plugin = make_ts_plugin(dir);
    let entry_path = plugin.entry_path.as_ref().expect("entry path");
    write(
        entry_path,
        br#"export default async function handler(ctx) {
  return { ok: true, output: `handler:${ctx.source}:${ctx.args.join("|")}` };
}
"#,
    )
    .expect("real handler entry");
    plugin
}

fn ctx(args: &[&str]) -> InvokeContext {
    InvokeContext {
        source: InvokeSource::Cli,
        args: args.iter().map(|arg| (*arg).to_owned()).collect(),
    }
}

#[test]
fn bun_invoke_runtime_calls_real_ts_default_export_handler() {
    let _guard = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if skip_without_bun("bun_invoke_runtime_calls_real_ts_default_export_handler") {
        return;
    }
    let _restore = EnvRestore::capture();
    let root = temp_dir("real-handler");
    let plugin_dir = root.join("plugin");
    create_dir_all(&plugin_dir).expect("plugin dir");
    let plugin = make_real_handler_plugin(&plugin_dir);

    let result = BunInvokeRuntime::default().invoke_ts(&plugin, &ctx(&["one", "two"]));

    assert_eq!(result, InvokeResult::output("handler:cli:one|two"));

    remove_dir_all(root).expect("cleanup");
}

#[test]
fn bun_invoke_runtime_passes_driver_args_stdin_context_and_parses_result() {
    let _guard = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if skip_without_bun("bun_invoke_runtime_passes_driver_args_stdin_context_and_parses_result") {
        return;
    }
    let _restore = EnvRestore::capture();
    let root = temp_dir("happy");
    let capture_dir = setup_fake_bun(&root);
    let plugin_dir = root.join("plugin");
    create_dir_all(&plugin_dir).expect("plugin dir");
    let plugin = make_ts_plugin(&plugin_dir);
    let entry_path = plugin.entry_path.as_ref().expect("entry path").clone();

    let result = BunInvokeRuntime::default().invoke_ts(&plugin, &ctx(&["one", "two"]));

    assert_eq!(result, InvokeResult::output("from bun"));
    assert_eq!(
        read_to_string(capture_dir.join("argv.txt")).expect("argv"),
        format!(
            "arg1=-e\narg3={}\narg4=one\narg5=two\n",
            entry_path.display()
        )
    );
    let context: serde_json::Value =
        serde_json::from_str(&read_to_string(capture_dir.join("stdin.json")).expect("stdin json"))
            .expect("valid context json");
    assert_eq!(context["source"], "cli");
    assert_eq!(context["args"], serde_json::json!(["one", "two"]));

    remove_dir_all(root).expect("cleanup");
}

#[test]
fn bun_invoke_runtime_maps_nonzero_exit_to_error() {
    let _guard = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if skip_without_bun("bun_invoke_runtime_maps_nonzero_exit_to_error") {
        return;
    }
    let _restore = EnvRestore::capture();
    let root = temp_dir("nonzero");
    setup_fake_bun(&root);
    std::env::set_var("MAW_BUN_MODE", "nonzero");
    let plugin_dir = root.join("plugin");
    create_dir_all(&plugin_dir).expect("plugin dir");
    let plugin = make_ts_plugin(&plugin_dir);

    let result = BunInvokeRuntime::default().invoke_ts(&plugin, &ctx(&[]));

    assert!(!result.ok);
    let error = result.error.as_deref().unwrap_or_default();
    assert!(error.contains("status 7"), "{error}");
    assert!(error.contains("boom"), "{error}");

    remove_dir_all(root).expect("cleanup");
}

#[test]
fn bun_invoke_runtime_reports_missing_bun() {
    let _guard = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _restore = EnvRestore::capture();
    let root = temp_dir("missing");
    let missing_path = root.join("missing-bin");
    create_dir_all(&missing_path).expect("missing bin dir");
    std::env::set_var("PATH", &missing_path);
    let plugin_dir = root.join("plugin");
    create_dir_all(&plugin_dir).expect("plugin dir");
    let plugin = make_ts_plugin(&plugin_dir);

    let result = BunInvokeRuntime::default().invoke_ts(&plugin, &ctx(&[]));

    assert!(!result.ok);
    let error = result.error.as_deref().unwrap_or_default();
    assert!(error.contains("failed to run bun"), "{error}");

    remove_dir_all(root).expect("cleanup");
}

#[test]
fn bun_invoke_runtime_reports_timeout() {
    let _guard = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if skip_without_bun("bun_invoke_runtime_reports_timeout") {
        return;
    }
    let _restore = EnvRestore::capture();
    let root = temp_dir("timeout");
    setup_fake_bun(&root);
    std::env::set_var("MAW_BUN_MODE", "timeout");
    let plugin_dir = root.join("plugin");
    create_dir_all(&plugin_dir).expect("plugin dir");
    let plugin = make_ts_plugin(&plugin_dir);

    let result =
        BunInvokeRuntime::with_timeout(Duration::from_millis(10)).invoke_ts(&plugin, &ctx(&[]));

    assert!(!result.ok);
    let error = result.error.as_deref().unwrap_or_default();
    assert!(error.contains("timed out"), "{error}");

    remove_dir_all(root).expect("cleanup");
}
