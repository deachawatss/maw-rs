use maw_cli::run_cli;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn run(args: &[String]) -> maw_cli::CliOutput {
    run_cli(args)
}

fn temp_plugin_root(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time after unix epoch")
        .as_nanos();
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("maw-rs-plugin-ls-{label}-{nonce}-{count}"));
    fs::create_dir_all(&root).expect("create temp plugin root");
    root
}

fn write_plugin(root: &Path, dir_name: &str, manifest: &str) {
    let dir = root.join(dir_name);
    fs::create_dir_all(&dir).expect("create plugin dir");
    fs::write(dir.join("index.ts"), "export function handle() {}\n").expect("write plugin entry");
    fs::write(dir.join("plugin.json"), manifest).expect("write plugin manifest");
}

#[test]
fn plugin_ls_groups_discovered_plugins_by_tier() {
    let root = temp_plugin_root("tiers");
    write_plugin(
        &root,
        "alpha",
        r#"{
          "name": "alpha",
          "version": "1.2.3",
          "sdk": "*",
          "tier": "standard",
          "entry": "index.ts",
          "cli": { "command": "alpha" },
          "api": { "path": "/api/plugins/alpha", "methods": ["GET"] }
        }"#,
    );
    write_plugin(
        &root,
        "bravo",
        r#"{
          "name": "bravo",
          "version": "0.2.0",
          "sdk": "*",
          "tier": "core",
          "entry": "index.ts",
          "cli": { "command": "bravo" }
        }"#,
    );
    write_plugin(
        &root,
        "charlie",
        r#"{
          "name": "charlie",
          "version": "0.3.0",
          "sdk": "*",
          "tier": "extra",
          "entry": "index.ts",
          "api": { "path": "/api/plugins/charlie", "methods": ["POST"] }
        }"#,
    );

    let output = run(&[
        "plugin".to_owned(),
        "ls".to_owned(),
        "--scan-dir".to_owned(),
        root.display().to_string(),
    ]);

    assert_eq!(output.code, 0, "{}", output.stderr);
    assert!(output.stdout.contains("core plugins\n"));
    assert!(output.stdout.contains("standard plugins\n"));
    assert!(output.stdout.contains("extra plugins\n"));
    assert!(output
        .stdout
        .contains("name     version  tier      surfaces"));
    assert!(output.stdout.contains("bravo    0.2.0    core      cli"));
    assert!(output
        .stdout
        .contains("alpha    1.2.3    standard  cli,api"));
    assert!(output.stdout.contains("charlie  0.3.0    extra     api"));
}

#[test]
fn plugin_ls_verbose_includes_details_and_warnings() {
    let root = temp_plugin_root("verbose");
    write_plugin(
        &root,
        "delta",
        r#"{
          "name": "delta",
          "version": "2.0.0",
          "sdk": "*",
          "entry": "index.ts",
          "description": "Delta tools",
          "weight": 7,
          "cli": { "command": "delta-tools" },
          "api": { "path": "/api/plugins/delta", "methods": ["GET", "POST"] }
        }"#,
    );
    write_plugin(
        &root,
        "future",
        r#"{
          "name": "future",
          "version": "9.0.0",
          "sdk": ">99.0.0",
          "entry": "index.ts",
          "tier": "extra"
        }"#,
    );

    let output = run(&[
        "plugin".to_owned(),
        "ls".to_owned(),
        "-v".to_owned(),
        "--scan-dir".to_owned(),
        root.display().to_string(),
        "--runtime-version".to_owned(),
        "1.0.0".to_owned(),
    ]);

    assert_eq!(output.code, 0, "{}", output.stderr);
    assert!(output.stdout.contains("delta  2.0.0    core  cli,api"));
    assert!(output.stdout.contains("  description: Delta tools"));
    assert!(output.stdout.contains("  kind: ts"));
    assert!(output.stdout.contains("  weight: 7"));
    assert!(output.stdout.contains("  cli: maw delta-tools"));
    assert!(output.stdout.contains("  api: /api/plugins/delta"));
    assert!(output.stdout.contains("warnings\n"));
    assert!(output
        .stdout
        .contains("plugin 'future' requires sdk >99.0.0"));
}

#[test]
fn plugin_ls_rejects_unknown_args() {
    let output = run(&["plugin".to_owned(), "ls".to_owned(), "--json".to_owned()]);

    assert_eq!(output.code, 2);
    assert!(output.stderr.contains("plugin ls: unknown argument --json"));
    assert!(output.stderr.contains("usage: maw-rs plugin ls"));
}
