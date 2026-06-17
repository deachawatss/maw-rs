use maw_cli::run_cli;
use serde_json::json;
use std::ffi::OsString;
use std::fs::{create_dir_all, remove_dir_all, write};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct EnvRestore {
    maw_plugins_dir: Option<OsString>,
}

impl EnvRestore {
    fn capture() -> Self {
        Self {
            maw_plugins_dir: std::env::var_os("MAW_PLUGINS_DIR"),
        }
    }
}

impl Drop for EnvRestore {
    fn drop(&mut self) {
        restore_env("MAW_PLUGINS_DIR", self.maw_plugins_dir.take());
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

#[test]
fn catch_all_dispatches_matching_plugin_cli_command_prefix() {
    let _guard = env_lock().lock().expect("env lock");
    let _restore = EnvRestore::capture();
    let root = temp_dir("prefix");
    let plugins_dir = root.join("plugins");
    let plugin_dir = plugins_dir.join("weather-demo");
    create_dir_all(&plugin_dir).expect("plugin dir");
    write(
        plugin_dir.join("index.ts"),
        b"export default async function plugin() {}\n",
    )
    .expect("entry");
    write(
        plugin_dir.join("plugin.json"),
        json!({
            "name": "weather-demo",
            "version": "1.0.0",
            "sdk": "*",
            "target": "js",
            "entry": "index.ts",
            "cli": {
                "command": "weather report",
                "help": "maw weather report [--city <name>]"
            }
        })
        .to_string(),
    )
    .expect("manifest");
    std::env::set_var("MAW_PLUGINS_DIR", &plugins_dir);

    let dispatched = run_cli(&args(&["weather", "report", "--help"]));

    assert_eq!(dispatched.code, 0, "{}", dispatched.stderr);
    assert!(dispatched.stderr.is_empty(), "{}", dispatched.stderr);
    assert!(dispatched.stdout.contains("weather-demo v1.0.0"));
    assert!(
        dispatched
            .stdout
            .contains("usage: maw weather report [--city <name>]"),
        "{}",
        dispatched.stdout
    );

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
