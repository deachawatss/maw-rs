use std::{fs, path::Path, process::Command};

fn temp_dir(label: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "maw-rs-plugin-build-{label}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("temp dir");
    path
}

fn copy_tree(src: &Path, dest: &Path) {
    fs::create_dir_all(dest).expect("create dest");
    for entry in fs::read_dir(src).expect("read fixture") {
        let entry = entry.expect("entry");
        let name = entry.file_name();
        if name == "target" {
            continue;
        }
        let src_path = entry.path();
        let dest_path = dest.join(name);
        if src_path.is_dir() {
            copy_tree(&src_path, &dest_path);
        } else {
            fs::copy(&src_path, &dest_path).expect("copy fixture file");
        }
    }
}

fn normalize_plugin_build_stdout(stdout: &str) -> String {
    let mut normalized = stdout
        .lines()
        .map(|line| {
            if line.trim_start().starts_with("sha256: sha256:") {
                "  sha256: sha256:cd5d4935a48c0672cb06407bb443bc0087aff947c6b864bac886982c73b3027f"
                    .to_owned()
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    normalized.push('\n');
    normalized
}

#[cfg(unix)]
fn write_executable(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;

    fs::write(path, body).expect("write executable");
    let mut permissions = fs::metadata(path).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("chmod executable");
}

#[test]
fn plugin_build_route_a_builds_dist_and_extism_loads_fixture() {
    let root = temp_dir("route-a");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/native-plugin-build/plugin-build-rust");
    let plugin_dir = root.join("plugin-build-rust");
    copy_tree(&fixture, &plugin_dir);

    let output = Command::new(env!("CARGO_BIN_EXE_maw-rs"))
        .args(["plugin", "build", plugin_dir.to_str().expect("utf8 path")])
        .env("MAW_JS_REF_DIR", "/nonexistent")
        .env("CARGO_TERM_COLOR", "never")
        .output()
        .expect("run maw-rs plugin build");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        normalize_plugin_build_stdout(&stdout),
        include_str!("fixtures/native-plugin-build/plugin-build-rust.stdout")
    );
    assert!(stdout.contains("sha256: sha256:"));
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());

    let wasm_path = plugin_dir.join("target/wasm32-unknown-unknown/release/route_probe.wasm");
    assert!(
        wasm_path.is_file(),
        "wasm should be produced at manifest wasm path"
    );
    let dist_wasm = plugin_dir.join("dist/plugin.wasm");
    assert!(dist_wasm.is_file(), "dist wasm should be emitted");
    let dist_manifest =
        fs::read_to_string(plugin_dir.join("dist/plugin.json")).expect("dist manifest");
    assert!(dist_manifest.contains(r#""artifact""#), "{dist_manifest}");
    assert!(dist_manifest.contains("sha256:"), "{dist_manifest}");
    assert!(
        dist_manifest.contains(r#""cli""#),
        "caps should be preserved: {dist_manifest}"
    );

    let plugin = maw_plugin_manifest::load_manifest_from_dir(&plugin_dir.join("dist"))
        .expect("load dist manifest")
        .expect("plugin loaded");
    assert_eq!(
        plugin
            .manifest
            .target
            .map(maw_plugin_manifest::PluginTarget::as_str),
        Some("wasm")
    );
    assert_eq!(plugin.kind.as_str(), "wasm");
    let mut runtime = maw_plugin_manifest::ExtismWasmInvokeRuntime::default();
    let result = maw_plugin_manifest::invoke_plugin(
        &plugin,
        &maw_plugin_manifest::InvokeContext {
            source: maw_plugin_manifest::InvokeSource::Cli,
            args: vec!["probe".to_owned()],
        },
        &mut runtime,
    );
    assert!(result.ok, "invoke error: {:?}", result.error);
    assert_eq!(result.output.as_deref(), Some("route-probe:called"));
}

#[test]
#[cfg(unix)]
fn plugin_build_ts_refusal_does_not_delegate_to_fake_maw_or_bun() {
    let root = temp_dir("fake-maw-proof");
    let bin = root.join("bin");
    fs::create_dir_all(&bin).expect("bin");
    write_executable(&bin.join("maw"), "#!/bin/sh\necho DELEGATED-MAW\nexit 37\n");
    write_executable(&bin.join("bun"), "#!/bin/sh\necho DELEGATED-BUN\nexit 38\n");

    let plugin_dir = root.join("ts-plugin");
    fs::create_dir_all(&plugin_dir).expect("plugin dir");
    fs::write(plugin_dir.join("index.ts"), "export function handle() {}\n").expect("entry");
    fs::write(
        plugin_dir.join("plugin.json"),
        r#"{"name":"ts-plugin","version":"1.0.0","sdk":"*","target":"js","entry":"index.ts"}"#,
    )
    .expect("manifest");

    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_maw-rs"))
        .args(["plugin", "build", plugin_dir.to_str().expect("utf8 path")])
        .env("PATH", path)
        .env("MAW_JS_REF_DIR", "/nonexistent")
        .output()
        .expect("run maw-rs plugin build");
    assert_eq!(output.status.code(), Some(2));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.is_empty(), "stdout: {stdout}");
    assert!(
        stderr.contains("JS/TS source builds are intentionally deferred"),
        "{stderr}"
    );
    assert!(!stdout.contains("DELEGATED-MAW"), "{stdout}");
    assert!(!stderr.contains("DELEGATED-MAW"), "{stderr}");
    assert!(!stdout.contains("DELEGATED-BUN"), "{stdout}");
    assert!(!stderr.contains("DELEGATED-BUN"), "{stderr}");
}
