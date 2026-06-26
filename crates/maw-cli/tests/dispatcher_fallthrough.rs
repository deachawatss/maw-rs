use maw_cli::{dispatcher_status, native_dispatch_commands, run_cli, DispatchKind};
use std::ffi::OsString;
use std::fs::{create_dir_all, remove_dir_all, write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct EnvRestore {
    maw_from_rs: Option<OsString>,
    maw_plugins_dir: Option<OsString>,
    maw_rs_hey_fallback: Option<OsString>,
    path: Option<OsString>,
}

impl EnvRestore {
    fn capture() -> Self {
        Self {
            maw_from_rs: std::env::var_os("MAW_FROM_RS"),
            maw_plugins_dir: std::env::var_os("MAW_PLUGINS_DIR"),
            maw_rs_hey_fallback: std::env::var_os("MAW_RS_HEY_FALLBACK"),
            path: std::env::var_os("PATH"),
        }
    }
}

impl Drop for EnvRestore {
    fn drop(&mut self) {
        restore_env("MAW_FROM_RS", self.maw_from_rs.take());
        restore_env("MAW_PLUGINS_DIR", self.maw_plugins_dir.take());
        restore_env("MAW_RS_HEY_FALLBACK", self.maw_rs_hey_fallback.take());
        restore_env("PATH", self.path.take());
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
        "maw-rs-dispatcher-fallthrough-{label}-{}-{nonce}",
        std::process::id()
    ));
    create_dir_all(&dir).expect("create temp dir");
    dir
}

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn write_maw_shim(dir: &Path, exit_code: u8) {
    let shim = dir.join("maw");
    write(
        &shim,
        format!(
            "#!/bin/sh\nprintf 'DELEGATED-MAW\\n'\nprintf 'MAW_FROM_RS=%s\\n' \"$MAW_FROM_RS\"\nprintf 'args=%s\\n' \"$*\"\nexit {exit_code}\n"
        ),
    )
    .expect("write maw shim");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&shim)
            .expect("shim metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&shim, permissions).expect("chmod maw shim");
    }
}

fn write_tool_shim(dir: &Path, name: &str, body: &str) {
    let shim = dir.join(name);
    write(&shim, body).expect("write tool shim");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&shim)
            .expect("tool shim metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&shim, permissions).expect("chmod tool shim");
    }
}

#[test]
fn dispatcher_table_marks_native_and_unknown_commands() {
    assert_eq!(dispatcher_status("ls"), DispatchKind::Native);
    assert_eq!(dispatcher_status("hey"), DispatchKind::Native);
    assert_eq!(dispatcher_status("check"), DispatchKind::Native);
    assert!(native_dispatch_commands().contains(&"ls"));
    assert!(native_dispatch_commands().contains(&"hey"));
    assert!(native_dispatch_commands().contains(&"check"));
}

#[test]
fn native_command_stays_native_without_invoking_maw_fallback() {
    let _guard = env_lock().lock().expect("env lock");
    let _restore = EnvRestore::capture();
    let root = temp_dir("native");
    let bin_dir = root.join("bin");
    let plugins_dir = root.join("plugins");
    create_dir_all(&bin_dir).expect("bin dir");
    create_dir_all(&plugins_dir).expect("plugins dir");
    write_maw_shim(&bin_dir, 42);
    std::env::set_var("PATH", &bin_dir);
    std::env::set_var("MAW_PLUGINS_DIR", &plugins_dir);
    std::env::remove_var("MAW_FROM_RS");

    let output = run_cli(&args(&["ls", "--help"]));

    assert_eq!(output.code, 0, "{}", output.stderr);
    assert!(output.stderr.is_empty(), "{}", output.stderr);
    assert!(output.stdout.contains("maw ls"), "{}", output.stdout);
    assert!(!output.stdout.contains("MAW_FROM_RS"), "{}", output.stdout);

    remove_dir_all(root).expect("cleanup");
}

#[test]
fn unknown_command_is_native_error_and_never_invokes_path_maw() {
    let _guard = env_lock().lock().expect("env lock");
    let _restore = EnvRestore::capture();
    let root = temp_dir("unknown-native");
    let bin_dir = root.join("bin");
    let plugins_dir = root.join("plugins");
    create_dir_all(&bin_dir).expect("bin dir");
    create_dir_all(&plugins_dir).expect("plugins dir");
    write_maw_shim(&bin_dir, 37);
    std::env::set_var("PATH", &bin_dir);
    std::env::set_var("MAW_PLUGINS_DIR", &plugins_dir);
    std::env::remove_var("MAW_FROM_RS");
    std::env::remove_var("MAW_RS_HEY_FALLBACK");

    let output = run_cli(&args(&["__definitely_unknown_xyz__"]));

    assert_eq!(
        dispatcher_status("__definitely_unknown_xyz__"),
        DispatchKind::NativeError
    );
    assert_eq!(output.code, 2, "{}", output.stdout);
    assert!(output.stdout.is_empty(), "{}", output.stdout);
    assert_eq!(
        output.stderr,
        include_str!("fixtures/zero-bun/unknown-command.stderr")
    );
    assert!(
        !output.stderr.contains("DELEGATED-MAW"),
        "{}",
        output.stderr
    );
    assert!(
        !output.stderr.contains("failed to run maw fallback"),
        "{}",
        output.stderr
    );

    remove_dir_all(root).expect("cleanup");
}

#[test]
fn hey_can_still_fall_through_to_maw_when_safety_env_is_set() {
    let _guard = env_lock().lock().expect("env lock");
    let _restore = EnvRestore::capture();
    let root = temp_dir("fallback");
    let bin_dir = root.join("bin");
    let plugins_dir = root.join("plugins");
    create_dir_all(&bin_dir).expect("bin dir");
    create_dir_all(&plugins_dir).expect("plugins dir");
    write_maw_shim(&bin_dir, 37);
    std::env::set_var("PATH", &bin_dir);
    std::env::set_var("MAW_PLUGINS_DIR", &plugins_dir);
    std::env::remove_var("MAW_FROM_RS");
    std::env::set_var("MAW_RS_HEY_FALLBACK", "1");

    let output = run_cli(&args(&["hey", "local:nova:claude", "ping"]));

    assert_eq!(output.code, 37, "{}", output.stderr);
    assert!(output.stderr.is_empty(), "{}", output.stderr);
    assert_eq!(
        output.stdout,
        "DELEGATED-MAW\nMAW_FROM_RS=1\nargs=hey local:nova:claude ping\n"
    );

    remove_dir_all(root).expect("cleanup");
}

#[test]
fn check_tools_is_native_and_never_invokes_path_maw() {
    let _guard = env_lock().lock().expect("env lock");
    let _restore = EnvRestore::capture();
    let root = temp_dir("check-tools-native");
    let bin_dir = root.join("bin");
    let plugins_dir = root.join("plugins");
    create_dir_all(&bin_dir).expect("bin dir");
    create_dir_all(&plugins_dir).expect("plugins dir");
    write_maw_shim(&bin_dir, 37);
    std::env::set_var("PATH", &bin_dir);
    std::env::set_var("MAW_PLUGINS_DIR", &plugins_dir);
    std::env::remove_var("MAW_FROM_RS");
    std::env::remove_var("MAW_RS_HEY_FALLBACK");

    let output = run_cli(&args(&["check", "tools"]));

    assert_eq!(dispatcher_status("check"), DispatchKind::Native);
    assert_eq!(output.code, 0, "{}", output.stderr);
    assert!(output.stderr.is_empty(), "{}", output.stderr);
    assert_eq!(
        output.stdout,
        include_str!("fixtures/zero-bun/check-tools-missing.stdout")
    );
    assert!(
        !output.stdout.contains("DELEGATED-MAW"),
        "{}",
        output.stdout
    );
    assert!(
        !output.stderr.contains("DELEGATED-MAW"),
        "{}",
        output.stderr
    );
    assert!(
        !output.stderr.contains("failed to run maw fallback"),
        "{}",
        output.stderr
    );

    remove_dir_all(root).expect("cleanup");
}

#[test]
fn check_tools_extracts_versions_with_argv_only_tool_probes() {
    let _guard = env_lock().lock().expect("env lock");
    let _restore = EnvRestore::capture();
    let root = temp_dir("check-tools-present");
    let bin_dir = root.join("bin");
    let plugins_dir = root.join("plugins");
    create_dir_all(&bin_dir).expect("bin dir");
    create_dir_all(&plugins_dir).expect("plugins dir");
    write_maw_shim(&bin_dir, 37);
    write_tool_shim(&bin_dir, "bun", "#!/bin/sh\nprintf 'bun 1.2.3\\n'\n");
    write_tool_shim(&bin_dir, "gh", "#!/bin/sh\nprintf 'gh version 2.3.4\\n'\n");
    write_tool_shim(&bin_dir, "ghq", "#!/bin/sh\nprintf 'ghq 3.4.5\\n'\n");
    write_tool_shim(
        &bin_dir,
        "git",
        "#!/bin/sh\nprintf 'git version 4.5.6\\n'\n",
    );
    write_tool_shim(&bin_dir, "tmux", "#!/bin/sh\nprintf 'tmux 5.6\\n'\n");
    write_tool_shim(&bin_dir, "uv", "#!/bin/sh\nprintf 'uv 6.7.8\\n'\n");
    write_tool_shim(
        &bin_dir,
        "uvx",
        "#!/bin/sh\nprintf 'uvx should-not-run\\n'\n",
    );
    write_tool_shim(
        &bin_dir,
        "which",
        "#!/bin/sh\nif [ \"$1\" = uvx ]; then printf 'uvx\\n'; exit 0; fi\nexit 1\n",
    );
    std::env::set_var("PATH", &bin_dir);
    std::env::set_var("MAW_PLUGINS_DIR", &plugins_dir);
    std::env::remove_var("MAW_FROM_RS");

    let output = run_cli(&args(&["check"]));

    assert_eq!(output.code, 0, "{}", output.stderr);
    assert!(output.stderr.is_empty(), "{}", output.stderr);
    assert!(
        output.stdout.contains("maw check tools"),
        "{}",
        output.stdout
    );
    assert!(
        output.stdout.contains("bun       1.2.3"),
        "{}",
        output.stdout
    );
    assert!(
        output.stdout.contains("gh        2.3.4"),
        "{}",
        output.stdout
    );
    assert!(
        output.stdout.contains("ghq       3.4.5"),
        "{}",
        output.stdout
    );
    assert!(
        output.stdout.contains("git       4.5.6"),
        "{}",
        output.stdout
    );
    assert!(output.stdout.contains("tmux      5.6"), "{}", output.stdout);
    assert!(
        output.stdout.contains("uv        6.7.8"),
        "{}",
        output.stdout
    );
    assert!(
        output.stdout.contains("uvx       6.7.8"),
        "{}",
        output.stdout
    );
    assert!(
        output.stdout.contains("(provided by uv)"),
        "{}",
        output.stdout
    );
    assert!(
        output
            .stdout
            .contains("5 required ✓  ·  2 optional ✓  ·  0 missing"),
        "{}",
        output.stdout
    );
    assert!(
        !output.stdout.contains("DELEGATED-MAW"),
        "{}",
        output.stdout
    );

    remove_dir_all(root).expect("cleanup");
}

#[test]
fn check_unknown_subcommand_matches_maw_js_usage_without_tool_exec() {
    let _guard = env_lock().lock().expect("env lock");
    let _restore = EnvRestore::capture();
    let root = temp_dir("check-unknown-subcommand");
    let bin_dir = root.join("bin");
    let plugins_dir = root.join("plugins");
    create_dir_all(&bin_dir).expect("bin dir");
    create_dir_all(&plugins_dir).expect("plugins dir");
    write_maw_shim(&bin_dir, 37);
    std::env::set_var("PATH", &bin_dir);
    std::env::set_var("MAW_PLUGINS_DIR", &plugins_dir);

    let output = run_cli(&args(&["check", "status"]));

    assert_eq!(output.code, 0, "{}", output.stderr);
    assert!(output.stderr.is_empty(), "{}", output.stderr);
    assert_eq!(
        output.stdout,
        "unknown subcommand: status\nusage: maw check [tools]\n"
    );
    assert!(
        !output.stdout.contains("DELEGATED-MAW"),
        "{}",
        output.stdout
    );

    remove_dir_all(root).expect("cleanup");
}

#[tokio::test]
async fn sync_async_handler_guard_refuses_to_block_inside_runtime() {
    let output = run_cli(&args(&["hey", "local:nova:claude", "ping"]));

    assert_ne!(output.code, 0);
    assert!(output.stdout.is_empty(), "{}", output.stdout);
    assert!(
        output.stderr.contains("cannot block_on inside runtime"),
        "{}",
        output.stderr
    );
}

#[test]
fn loop_guard_returns_unknown_command_instead_of_falling_through() {
    let _guard = env_lock().lock().expect("env lock");
    let _restore = EnvRestore::capture();
    let root = temp_dir("loop-guard");
    let bin_dir = root.join("bin");
    let plugins_dir = root.join("plugins");
    create_dir_all(&bin_dir).expect("bin dir");
    create_dir_all(&plugins_dir).expect("plugins dir");
    write_maw_shim(&bin_dir, 37);
    std::env::set_var("PATH", &bin_dir);
    std::env::set_var("MAW_PLUGINS_DIR", &plugins_dir);
    std::env::set_var("MAW_FROM_RS", "1");
    std::env::set_var("MAW_RS_HEY_FALLBACK", "1");

    let output = run_cli(&args(&["hey", "local:nova:claude", "ping"]));

    assert_eq!(output.code, 2, "{}", output.stdout);
    assert!(output.stdout.is_empty(), "{}", output.stdout);
    assert!(
        output.stderr.contains("maw-rs: unknown command 'hey'"),
        "{}",
        output.stderr
    );

    remove_dir_all(root).expect("cleanup");
}

#[test]
fn plugin_manifest_invoke_is_native_and_never_invokes_path_maw() {
    let _guard = env_lock().lock().expect("env lock");
    let _restore = EnvRestore::capture();
    let root = temp_dir("plugin-manifest-native");
    let bin_dir = root.join("bin");
    let plugins_dir = root.join("plugins");
    let manifest_dir = plugins_dir.join("ts-proof");
    create_dir_all(&bin_dir).expect("bin dir");
    create_dir_all(&manifest_dir).expect("plugin dir");
    write_maw_shim(&bin_dir, 37);
    write(
        manifest_dir.join("plugin.json"),
        r#"{"name":"ts-proof","version":"1.0.0","sdk":"*","target":"js","entry":"index.ts"}"#,
    )
    .expect("manifest");
    write(
        manifest_dir.join("index.ts"),
        b"export default () => ({ ok: true });\n",
    )
    .expect("entry");
    std::env::set_var("PATH", &bin_dir);
    std::env::set_var("MAW_PLUGINS_DIR", &plugins_dir);
    std::env::set_var("MAW_JS_REF_DIR", "/nonexistent");
    std::env::remove_var("MAW_FROM_RS");
    std::env::remove_var("MAW_RS_HEY_FALLBACK");

    let output = run_cli(&args(&[
        "plugin-manifest",
        "invoke",
        "--scan-dir",
        plugins_dir.to_str().expect("plugins path"),
        "--plugin",
        "ts-proof",
    ]));

    assert_eq!(dispatcher_status("plugin-manifest"), DispatchKind::Native);
    assert_eq!(output.code, 2, "{}", output.stdout);
    assert!(output.stdout.is_empty(), "{}", output.stdout);
    assert!(
        output
            .stderr
            .contains("No Bun/JS subprocess fallback is available"),
        "{}",
        output.stderr
    );
    assert!(
        !output.stdout.contains("DELEGATED-MAW"),
        "{}",
        output.stdout
    );
    assert!(
        !output.stderr.contains("DELEGATED-MAW"),
        "{}",
        output.stderr
    );

    remove_dir_all(root).expect("cleanup");
}
