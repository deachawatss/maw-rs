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

#[test]
fn plugin_build_route_a_builds_and_extism_loads_fixture() {
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
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        include_str!("fixtures/native-plugin-build/plugin-build-rust.stdout")
    );
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());

    let wasm_path = plugin_dir.join("target/wasm32-unknown-unknown/release/route_probe.wasm");
    assert!(
        wasm_path.is_file(),
        "wasm should be produced at manifest wasm path"
    );
    let plugin = maw_plugin_manifest::load_manifest_from_dir(&plugin_dir)
        .expect("load manifest")
        .expect("plugin loaded");
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
