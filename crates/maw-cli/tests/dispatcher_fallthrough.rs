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
    path: Option<OsString>,
}

impl EnvRestore {
    fn capture() -> Self {
        Self {
            maw_from_rs: std::env::var_os("MAW_FROM_RS"),
            maw_plugins_dir: std::env::var_os("MAW_PLUGINS_DIR"),
            path: std::env::var_os("PATH"),
        }
    }
}

impl Drop for EnvRestore {
    fn drop(&mut self) {
        restore_env("MAW_FROM_RS", self.maw_from_rs.take());
        restore_env("MAW_PLUGINS_DIR", self.maw_plugins_dir.take());
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
            "#!/bin/sh\nprintf 'MAW_FROM_RS=%s\\n' \"$MAW_FROM_RS\"\nprintf 'args=%s\\n' \"$*\"\nexit {exit_code}\n"
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

#[test]
fn dispatcher_table_marks_native_and_fallback_commands() {
    assert_eq!(dispatcher_status("ls"), DispatchKind::Native);
    assert_eq!(dispatcher_status("hey"), DispatchKind::BunFallback);
    assert!(native_dispatch_commands().contains(&"ls"));
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
fn unported_command_falls_through_to_maw_with_env_args_and_exit_code() {
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

    let output = run_cli(&args(&["hey", "local:nova:claude", "ping"]));

    assert_eq!(output.code, 37, "{}", output.stderr);
    assert!(output.stderr.is_empty(), "{}", output.stderr);
    assert_eq!(
        output.stdout,
        "MAW_FROM_RS=1\nargs=hey local:nova:claude ping\n"
    );

    remove_dir_all(root).expect("cleanup");
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

    let output = run_cli(&args(&["hey", "local:nova:claude", "ping"]));

    assert_eq!(output.code, 2, "{}", output.stdout);
    assert!(output.stdout.is_empty(), "{}", output.stdout);
    assert!(
        output.stderr.contains("unknown command: hey"),
        "{}",
        output.stderr
    );

    remove_dir_all(root).expect("cleanup");
}
