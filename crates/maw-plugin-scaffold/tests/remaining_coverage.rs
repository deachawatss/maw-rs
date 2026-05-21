use maw_plugin_scaffold::{scaffold_as, scaffold_rust, validate_plugin_name};
use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

fn temp_dir(name: &str) -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("maw-plugin-scaffold-{name}-{nonce}"))
}

#[test]
fn scaffold_rust_writes_manifest_and_readme() {
    let root = temp_dir("rust");
    let template = root.join("template");
    let dest = root.join("dest");
    fs::create_dir_all(&template).expect("create template");
    fs::write(
        template.join("Cargo.toml"),
        r#"[package]
name = "template"
[dependencies]
maw-plugin-sdk = { path = "old" }
"#,
    )
    .expect("write cargo template");

    scaffold_rust("hello-world", &dest, &template, "../sdk").expect("scaffold rust");

    assert!(fs::read_to_string(dest.join("plugin.json"))
        .expect("read manifest")
        .contains("hello-world"));
    assert!(fs::read_to_string(dest.join("README.md"))
        .expect("read readme")
        .contains("hello-world"));
    assert!(fs::read_to_string(dest.join("Cargo.toml"))
        .expect("read cargo")
        .contains(r#"maw-plugin-sdk = { path = "../sdk" }"#));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn scaffold_as_writes_manifest_and_rewrites_package() {
    let root = temp_dir("as");
    let template = root.join("template");
    let dest = root.join("dest");
    fs::create_dir_all(&template).expect("create template");
    fs::write(template.join("package.json"), r#"{"name":"template"}"#)
        .expect("write package template");

    scaffold_as("hello_as", &dest, &template).expect("scaffold as");

    assert!(fs::read_to_string(dest.join("plugin.json"))
        .expect("read manifest")
        .contains("hello-as"));
    assert!(fs::read_to_string(dest.join("README.md"))
        .expect("read readme")
        .contains("hello_as"));
    assert!(fs::read_to_string(dest.join("package.json"))
        .expect("read package")
        .contains("hello_as"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn empty_plugin_name_fails_validator() {
    assert_eq!(
        validate_plugin_name(""),
        Some("name is required".to_owned())
    );
}
