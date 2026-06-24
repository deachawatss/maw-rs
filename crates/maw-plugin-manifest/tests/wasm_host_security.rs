use std::fs::{create_dir_all, read_to_string, write};
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use maw_plugin_manifest::{parse_manifest, HostErrorCode, MawWasmHost, PluginManifest};
use serde_json::{json, Value};

fn temp(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "maw-wasm-host-{label}-{}-{nonce}",
        std::process::id()
    ));
    create_dir_all(&dir).expect("temp dir");
    dir
}

fn manifest(dir: &Path, caps: &[&str]) -> PluginManifest {
    write(dir.join("plugin.wasm"), b"\0asm\x01\0\0\0").expect("wasm");
    parse_manifest(
        &json!({
            "name": "secure-plugin",
            "version": "1.0.0",
            "sdk": "*",
            "entry": { "kind": "wasm", "path": "plugin.wasm", "export": "handle" },
            "capabilities": caps,
        })
        .to_string(),
        dir,
    )
    .expect("manifest")
}

fn host(dir: &Path, caps: &[&str]) -> MawWasmHost {
    let manifest = manifest(dir, caps);
    let loaded = maw_plugin_manifest::LoadedPlugin {
        manifest,
        dir: dir.to_path_buf(),
        wasm_path: dir.join("plugin.wasm"),
        entry_path: None,
        kind: maw_plugin_manifest::LoadedPluginKind::Wasm,
        disabled: false,
    };
    MawWasmHost::new(&loaded).with_fs_root("sandbox", dir)
}

fn call(host: &MawWasmHost, name: &str, args: &Value) -> Value {
    serde_json::from_str(&host.handle_json(name, &args.to_string())).expect("host result json")
}

#[test]
fn manifest_accepts_entry_object_wasm_form() {
    let dir = temp("entry-object");
    let parsed = manifest(&dir, &["fs:read:sandbox"]);
    assert_eq!(parsed.entry.as_deref(), Some("plugin.wasm"));
    assert_eq!(parsed.target, None);
}

#[test]
fn fs_read_denies_symlink_escape_and_proc() {
    let dir = temp("symlink");
    write(dir.join("safe.txt"), "ok").expect("safe");
    symlink("/etc/passwd", dir.join("escape")).expect("symlink");
    let host = host(&dir, &["fs:read:sandbox"]);

    let safe = call(&host, "maw.fs.read", &json!({"path": dir.join("safe.txt")}));
    assert_eq!(safe["ok"], true);
    assert_eq!(safe["value"]["content"], "ok");

    let escaped = call(&host, "maw.fs.read", &json!({"path": dir.join("escape")}));
    assert_eq!(escaped["ok"], false);
    assert_eq!(escaped["code"], "capability_denied");

    let proc = call(&host, "maw.fs.read", &json!({"path": "/proc/self/cmdline"}));
    assert_eq!(proc["ok"], false);
}

#[test]
fn fs_write_uses_nofollow_and_denies_existing_symlink() {
    let dir = temp("write-symlink");
    let outside = temp("outside").join("pwned.txt");
    write(&outside, "outside").expect("outside");
    symlink(&outside, dir.join("link.txt")).expect("symlink");
    let host = host(&dir, &["fs:write:sandbox"]);

    let denied = call(
        &host,
        "maw.fs.write",
        &json!({"path": dir.join("link.txt"), "content": "secret" , "mode": "overwrite"}),
    );
    assert_eq!(denied["ok"], false);
    assert_eq!(
        read_to_string(&outside).expect("outside unchanged"),
        "outside"
    );
}

#[test]
fn secret_bytes_are_redacted_from_audit_and_headers() {
    let dir = temp("redact");
    let host = host(&dir, &["net:https:example.com"]);
    let result = call(
        &host,
        "maw.http.request",
        &json!({
            "method": "GET",
            "url": "https://example.com/secret-token-value",
            "headers": { "Authorization": "peerKey-secret-token-value" },
            "timeoutMs": 1
        }),
    );
    assert_eq!(result["ok"], false);
    let audit = host.audit_json_lines();
    assert!(
        !audit.contains("peerKey-secret-token-value"),
        "audit leaked secret: {audit}"
    );
    assert!(
        !audit.contains("Authorization"),
        "audit leaked header name/value: {audit}"
    );
}

#[test]
fn exec_enforces_capability_and_env_allowlist() {
    let dir = temp("exec");
    let host = host(&dir, &["proc:exec:env", "fs:read:sandbox"]);
    let denied_env = call(
        &host,
        "maw.exec.run",
        &json!({
            "cmd": "env",
            "cwd": dir,
            "env": { "SECRET_TOKEN": "do-not-pass" },
            "allowNonZero": true
        }),
    );
    assert_eq!(denied_env["ok"], false);
    assert_eq!(denied_env["code"], "capability_denied");

    let out = call(
        &host,
        "maw.exec.run",
        &json!({
            "cmd": "env",
            "cwd": dir,
            "env": { "MAW_VISIBLE": "yes", "HOME": "/should/not/inherit" },
            "allowNonZero": true
        }),
    );
    assert_eq!(out["ok"], true);
    let stdout = out["value"]["stdout"].as_str().unwrap_or_default();
    assert!(stdout.contains("MAW_VISIBLE=yes"));
    assert!(!stdout.contains("HOME=/should/not/inherit"));
}

#[test]
fn capability_denied_uses_error_envelope_and_private_net_hard_deny() {
    let dir = temp("cap-deny");
    let host = host(&dir, &["fs:read:sandbox", "net:http:127.0.0.1"]);
    let fs = call(
        &host,
        "maw.fs.write",
        &json!({"path": dir.join("x"), "content": "x"}),
    );
    assert_eq!(fs["ok"], false);
    assert_eq!(fs["code"], "capability_denied");

    let http = call(
        &host,
        "maw.http.request",
        &json!({"method": "GET", "url": "http://127.0.0.1/"}),
    );
    assert_eq!(http["ok"], false);
    assert_eq!(http["code"], "capability_denied");
}

#[test]
fn ipv4_mapped_ipv6_private_hosts_are_denied() {
    let dir = temp("ipv4-mapped");
    let host = host(
        &dir,
        &[
            "net:http:::ffff:127.0.0.1",
            "net:http:::ffff:169.254.169.254",
        ],
    );

    for url in [
        "http://[::ffff:127.0.0.1]/",
        "http://[::ffff:169.254.169.254]/",
    ] {
        let result = call(
            &host,
            "maw.http.request",
            &json!({"method": "GET", "url": url}),
        );
        assert_eq!(result["ok"], false, "{url}");
        assert_eq!(result["code"], "capability_denied", "{url}");
    }
}

#[test]
fn hard_denies_sudo_independent_of_manifest() {
    let dir = temp("sudo");
    let host = host(
        &dir,
        &[
            "proc:exec:sudo",
            "proc:exec:su",
            "proc:exec:doas",
            "proc:exec:pkexec",
            "fs:read:sandbox",
        ],
    );
    for cmd in ["sudo", "su", "doas", "pkexec"] {
        let result = call(
            &host,
            "maw.exec.run",
            &json!({"cmd": cmd, "args": ["id"], "cwd": dir}),
        );
        assert_eq!(result["ok"], false, "{cmd}");
        assert_eq!(result["code"], "capability_denied", "{cmd}");
    }
}

#[test]
fn host_error_code_serializes_contract_labels() {
    assert_eq!(
        serde_json::to_value(HostErrorCode::CapabilityDenied).unwrap(),
        "capability_denied"
    );
}
