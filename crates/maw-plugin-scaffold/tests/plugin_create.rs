use maw_plugin_scaffold::{build_manifest_json, validate_plugin_name, PluginLanguage};
use serde_json::Value;

#[test]
fn validate_plugin_name_accepts_simple_lowercase_name() {
    assert_eq!(validate_plugin_name("hello"), None);
}

#[test]
fn validate_plugin_name_accepts_name_with_hyphens_and_digits() {
    assert_eq!(validate_plugin_name("my-plugin-2"), None);
}

#[test]
fn validate_plugin_name_accepts_name_with_underscores() {
    assert_eq!(validate_plugin_name("my_plugin"), None);
}

#[test]
fn validate_plugin_name_rejects_empty_string() {
    assert!(validate_plugin_name("").is_some());
}

#[test]
fn validate_plugin_name_rejects_name_starting_with_digit() {
    assert!(validate_plugin_name("2plugin").is_some());
}

#[test]
fn validate_plugin_name_rejects_name_with_uppercase_letters() {
    assert!(validate_plugin_name("MyPlugin").is_some());
}

#[test]
fn validate_plugin_name_rejects_name_with_spaces() {
    assert!(validate_plugin_name("my plugin").is_some());
}

#[test]
fn rust_manifest_matches_scaffolded_plugin_json_contract() {
    let data = manifest("my-rust-plugin", PluginLanguage::Rust);

    assert_eq!(data["name"], "my-rust-plugin");
    assert_eq!(data["version"], "0.1.0");
    assert_eq!(data["sdk"], "^1.0.0");
    assert_eq!(
        data["wasm"],
        "./target/wasm32-unknown-unknown/release/my_rust_plugin.wasm"
    );
    assert_eq!(data["cli"]["command"], "my-rust-plugin");
    assert_eq!(data["api"]["path"], "/api/plugins/my-rust-plugin");
}

#[test]
fn assemblyscript_manifest_matches_scaffolded_plugin_json_contract() {
    let data = manifest("my-as-plugin", PluginLanguage::AssemblyScript);

    assert_eq!(data["name"], "my-as-plugin");
    assert_eq!(data["version"], "0.1.0");
    assert_eq!(data["sdk"], "^1.0.0");
    assert_eq!(data["wasm"], "./build/release.wasm");
    assert_eq!(data["cli"]["command"], "my-as-plugin");
    assert_eq!(data["api"]["path"], "/api/plugins/my-as-plugin");
}

#[test]
fn build_manifest_json_normalizes_underscores_to_hyphens_in_slug_fields() {
    let data = manifest("my_plugin", PluginLanguage::Rust);

    assert_eq!(data["name"], "my-plugin");
    assert!(data["wasm"]
        .as_str()
        .expect("wasm string")
        .contains("my_plugin.wasm"));
    assert_eq!(data["cli"]["command"], "my-plugin");
    assert_eq!(data["api"]["path"], "/api/plugins/my-plugin");
    assert_eq!(data["api"]["methods"], serde_json::json!(["GET", "POST"]));
}

#[test]
fn build_manifest_json_ends_with_newline() {
    assert!(build_manifest_json("my-plugin", PluginLanguage::Rust).ends_with('\n'));
}

fn manifest(name: &str, lang: PluginLanguage) -> Value {
    serde_json::from_str(&build_manifest_json(name, lang)).expect("valid manifest json")
}
