use maw_cli::run_cli;
use serde_json::json;
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
    home: Option<OsString>,
    maw_home: Option<OsString>,
    maw_plugins_dir: Option<OsString>,
    path: Option<OsString>,
}

impl EnvRestore {
    fn capture() -> Self {
        Self {
            home: std::env::var_os("HOME"),
            maw_home: std::env::var_os("MAW_HOME"),
            maw_plugins_dir: std::env::var_os("MAW_PLUGINS_DIR"),
            path: std::env::var_os("PATH"),
        }
    }
}

impl Drop for EnvRestore {
    fn drop(&mut self) {
        restore_env("HOME", self.home.take());
        restore_env("MAW_HOME", self.maw_home.take());
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
        "maw-rs-cli-plugin-dispatch-{label}-{}-{nonce}",
        std::process::id()
    ));
    create_dir_all(&dir).expect("create temp dir");
    dir
}

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn write_maw_shim(dir: &Path) {
    let shim = dir.join("maw");
    write(
        &shim,
        "#!/bin/sh\nprintf 'MAW_FROM_RS=%s\\n' \"$MAW_FROM_RS\"\nprintf 'args=%s\\n' \"$*\"\n",
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

fn write_ts_plugin(plugins_dir: &Path, dir_name: &str, command: &str) {
    let package_dir = plugins_dir.join(dir_name);
    create_dir_all(&package_dir).expect("plugin dir");
    write(
        package_dir.join("index.ts"),
        b"export default async function plugin() {}\n",
    )
    .expect("entry");
    write(
        package_dir.join("plugin.json"),
        json!({
            "name": dir_name,
            "version": "1.0.0",
            "sdk": "*",
            "target": "js",
            "entry": "index.ts",
            "cli": {
                "command": command,
                "help": format!("maw {command}")
            }
        })
        .to_string(),
    )
    .expect("manifest");
}

#[test]
fn dispatch_cli_plugin_finds_matching_plugin_and_runs_maw_bridge() {
    let _guard = env_lock().lock().expect("env lock");
    let _restore = EnvRestore::capture();
    let root = temp_dir("prefix");
    let bin_dir = root.join("bin");
    let plugins_dir = root.join("plugins");
    create_dir_all(&bin_dir).expect("bin dir");
    create_dir_all(&plugins_dir).expect("plugins dir");
    write_maw_shim(&bin_dir);
    write_ts_plugin(&plugins_dir, "weather-demo", "weather report");
    std::env::set_var("PATH", &bin_dir);
    std::env::set_var("MAW_PLUGINS_DIR", &plugins_dir);

    let dispatched = run_cli(&args(&["weather", "report", "--city", "Bangkok"]));

    assert_eq!(dispatched.code, 0, "{}", dispatched.stderr);
    assert!(dispatched.stderr.is_empty(), "{}", dispatched.stderr);
    assert_eq!(
        dispatched.stdout,
        "MAW_FROM_RS=1\nargs=weather report --city Bangkok\n"
    );

    remove_dir_all(root).expect("cleanup");
}

#[test]
fn unknown_plugin_command_falls_through_to_cli_error() {
    let _guard = env_lock().lock().expect("env lock");
    let _restore = EnvRestore::capture();
    let root = temp_dir("unknown");
    let bin_dir = root.join("bin");
    let plugins_dir = root.join("plugins");
    create_dir_all(&bin_dir).expect("bin dir");
    create_dir_all(&plugins_dir).expect("plugins dir");
    write_maw_shim(&bin_dir);
    write_ts_plugin(&plugins_dir, "weather-demo", "weather report");
    std::env::set_var("PATH", &bin_dir);
    std::env::set_var("MAW_PLUGINS_DIR", &plugins_dir);

    let partial = run_cli(&args(&["weather", "--help"]));

    assert_eq!(partial.code, 2, "{}", partial.stdout);
    assert!(partial.stdout.is_empty(), "{}", partial.stdout);
    assert!(
        partial.stderr.contains("unknown command: weather"),
        "{}",
        partial.stderr
    );

    remove_dir_all(root).expect("cleanup");
}

#[test]
fn plugin_ls_scans_home_maw_plugins_by_default() {
    let _guard = env_lock().lock().expect("env lock");
    let _restore = EnvRestore::capture();
    let root = temp_dir("home-scan");
    let home = root.join("home");
    let plugins_dir = home.join(".maw").join("plugins");
    create_dir_all(&plugins_dir).expect("home plugins dir");
    write_ts_plugin(&plugins_dir, "home-weather", "home weather");
    std::env::set_var("HOME", &home);
    std::env::remove_var("MAW_HOME");
    std::env::remove_var("MAW_PLUGINS_DIR");

    let output = run_cli(&args(&["plugin", "ls"]));

    assert_eq!(output.code, 0, "{}", output.stderr);
    assert!(output.stderr.is_empty(), "{}", output.stderr);
    assert_eq!(
        output.stdout,
        "1 plugin (1 active, 0 disabled)\n  core: 0 · standard: 0 · extra: 1\n  cli: 1 · api: 0 · health: ok\n"
    );

    remove_dir_all(root).expect("cleanup");
}
