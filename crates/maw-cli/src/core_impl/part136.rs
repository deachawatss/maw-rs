const DISPATCH_136: &[DispatcherEntry] = &[DispatcherEntry {
    command: "config",
    handler: Handler::Sync(config_run_command),
}];

const CONFIG_USAGE: &str =
    "usage: maw config <show|sources|explain <key>|set <key> <value>> [--json]";

fn config_run_command(argv: &[String]) -> CliOutput {
    match config_dispatch(argv) {
        Ok(stdout) => CliOutput {
            code: 0,
            stdout,
            stderr: String::new(),
        },
        Err(message) => CliOutput {
            code: 1,
            stdout: String::new(),
            stderr: format!("{message}\n"),
        },
    }
}

fn config_dispatch(argv: &[String]) -> Result<String, String> {
    let sub = argv.first().map_or("show", String::as_str);
    let json = argv.iter().any(|arg| arg == "--json");
    let reveal = argv.iter().any(|arg| arg == "--reveal");
    match sub {
        "set" => config_set(argv, json),
        "show" => config_show(reveal),
        _ => Err(CONFIG_USAGE.to_owned()),
    }
}

fn config_set(argv: &[String], json: bool) -> Result<String, String> {
    let key = argv
        .get(1)
        .map(String::as_str)
        .ok_or_else(|| "usage: maw config set <key> <value>".to_owned())?;
    if key.starts_with('-') {
        return Err("usage: maw config set <key> <value>".to_owned());
    }
    let raw = argv
        .get(2)
        .map(String::as_str)
        .ok_or_else(|| "usage: maw config set <key> <value>".to_owned())?;
    let value = match key {
        "node" => serde_json::Value::String(config_validate_node(raw)?),
        "port" => serde_json::Value::Number(config_validate_port(raw)?.into()),
        _ => return Err("maw config: native set currently supports node|port".to_owned()),
    };
    let mut config = config_read_target()?;
    let object = config
        .as_object_mut()
        .ok_or_else(|| "maw config: root config must be a JSON object".to_owned())?;
    object.insert(key.to_owned(), value.clone());
    config_atomic_write(
        &config_target_path(),
        &format!(
            "{}\n",
            serde_json::to_string_pretty(&config)
                .map_err(|error| format!("maw config: failed to render JSON: {error}"))?
        ),
    )?;
    if json {
        Ok(format!(
            "{}\n",
            serde_json::to_string_pretty(&serde_json::json!({ "key": key, "value": value }))
                .map_err(|error| format!("maw config: failed to render JSON: {error}"))?
        ))
    } else {
        Ok(format!(
            "{key} = {}\n",
            serde_json::to_string(&value)
                .map_err(|error| format!("maw config: failed to render value: {error}"))?
        ))
    }
}

fn config_show(reveal: bool) -> Result<String, String> {
    let mut config = config_read_target()?;
    if !reveal {
        config_redact_value(&mut config);
    }
    serde_json::to_string_pretty(&config)
        .map(|body| format!("{body}\n"))
        .map_err(|error| format!("maw config: failed to render JSON: {error}"))
}

fn config_redact_value(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if config_is_secret_key(key) {
                    *child = config_mask_secret(child);
                } else {
                    config_redact_value(child);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                config_redact_value(item);
            }
        }
        _ => {}
    }
}

fn config_mask_secret(value: &serde_json::Value) -> serde_json::Value {
    let Some(raw) = value.as_str() else {
        return serde_json::Value::String("****".to_owned());
    };
    if raw.chars().count() <= 4 {
        return serde_json::Value::String("****".to_owned());
    }
    let tail: String = raw
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    serde_json::Value::String(format!("****...{tail}"))
}

fn config_is_secret_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "node" | "port" | "host" | "url" | "oracleurl" | "bind" | "keyprefix"
    ) {
        return false;
    }
    if lower == "federationtoken"
        || lower == "pubkey"
        || lower == "peerpubkey"
        || lower == "peerkey"
    {
        return true;
    }
    lower.contains("token")
        || lower.contains("secret")
        || lower.contains("password")
        || lower.contains("key")
}

fn config_read_target() -> Result<serde_json::Value, String> {
    let path = config_target_path();
    if !path.exists() {
        return Ok(serde_json::json!({}));
    }
    let raw = std::fs::read_to_string(&path)
        .map_err(|error| format!("maw config: failed to read config: {error}"))?;
    serde_json::from_str::<serde_json::Value>(&raw)
        .map_err(|error| format!("maw config: failed to parse config JSON: {error}"))
}

fn config_target_path() -> std::path::PathBuf {
    let env = current_xdg_env();
    let weighted = maw_config_path(&env, &["maw.config.50.json"]);
    if weighted.exists() {
        weighted
    } else {
        maw_config_path(&env, &["maw.config.json"])
    }
}

fn config_atomic_write(path: &std::path::Path, body: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("maw config: failed to create config dir: {error}"))?;
    }
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("maw.config.json");
    let tmp = path.with_file_name(format!("{file_name}.tmp"));
    std::fs::write(&tmp, body)
        .map_err(|error| format!("maw config: failed to write temp file: {error}"))?;
    std::fs::rename(&tmp, path)
        .map_err(|error| format!("maw config: failed to replace config: {error}"))
}

fn config_validate_node(raw: &str) -> Result<String, String> {
    let value = raw.trim();
    if value.is_empty()
        || value.len() > 64
        || value.starts_with('-')
        || value.contains('/')
        || value.contains('\\')
        || value.chars().any(char::is_control)
    {
        return Err("maw config: invalid node name".to_owned());
    }
    if !value
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_')
    {
        return Err("maw config: invalid node name".to_owned());
    }
    Ok(value.to_owned())
}

fn config_validate_port(raw: &str) -> Result<u16, String> {
    if raw.starts_with('-') || raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("maw config: invalid port (expected 1-65535)".to_owned());
    }
    let port = raw
        .parse::<u16>()
        .map_err(|_| "maw config: invalid port (expected 1-65535)".to_owned())?;
    if port == 0 {
        return Err("maw config: invalid port (expected 1-65535)".to_owned());
    }
    Ok(port)
}

#[cfg(test)]
mod config_tests {
    use super::{
        config_dispatch, config_target_path, dispatcher_status, DispatchKind, EnvVarRestore,
    };

    #[test]
    fn config_dispatch_registers_native() {
        assert_eq!(dispatcher_status("config"), DispatchKind::Native);
    }

    #[test]
    fn config_rejects_bad_node_and_port_before_write() {
        assert!(
            config_dispatch(&["set".to_owned(), "node".to_owned(), "--bad".to_owned()])
                .expect_err("bad node")
                .contains("invalid node")
        );
        assert!(
            config_dispatch(&["set".to_owned(), "port".to_owned(), "0".to_owned()])
                .expect_err("bad port")
                .contains("invalid port")
        );
    }

    #[test]
    fn config_set_uses_weighted_target_when_present() {
        let _lock = super::env_test_lock().lock().expect("env lock");
        let _home = EnvVarRestore::capture("MAW_HOME");
        let _config = EnvVarRestore::capture("MAW_CONFIG_DIR");
        let root = std::env::temp_dir().join(format!("maw-rs-config-unit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::env::remove_var("MAW_HOME");
        std::env::set_var("MAW_CONFIG_DIR", root.join("config"));
        std::fs::create_dir_all(root.join("config")).expect("config dir");
        std::fs::write(
            root.join("config/maw.config.50.json"),
            "{\"node\":\"old\"}\n",
        )
        .expect("seed");
        assert_eq!(config_target_path(), root.join("config/maw.config.50.json"));
        let stdout = config_dispatch(&["set".to_owned(), "node".to_owned(), "new-node".to_owned()])
            .expect("set node");
        assert_eq!(stdout, "node = \"new-node\"\n");
        let body =
            std::fs::read_to_string(root.join("config/maw.config.50.json")).expect("config body");
        assert!(body.contains("\"node\": \"new-node\""));
        assert!(!root.join("config/maw.config.50.json.tmp").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn config_show_redacts_federation_token_by_default() {
        let _lock = super::env_test_lock().lock().expect("env lock");
        let _home = EnvVarRestore::capture("MAW_HOME");
        let _config = EnvVarRestore::capture("MAW_CONFIG_DIR");
        let root = config_seed_secret_fixture("redact-default");
        let stdout = config_dispatch(&["show".to_owned()]).expect("show");
        assert!(stdout.contains("\"federationToken\": \"****...1234\""));
        assert!(!stdout.contains("super-secret-token-1234"));
        assert!(stdout.contains("\"node\": \"bigboy-vps\""));
        assert!(stdout.contains("\"port\": 3456"));
        assert!(stdout.contains("\"url\": \"https://peer.example/api\""));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn config_show_reveal_prints_raw_secret_when_explicit() {
        let _lock = super::env_test_lock().lock().expect("env lock");
        let _home = EnvVarRestore::capture("MAW_HOME");
        let _config = EnvVarRestore::capture("MAW_CONFIG_DIR");
        let root = config_seed_secret_fixture("reveal");
        let stdout =
            config_dispatch(&["show".to_owned(), "--reveal".to_owned()]).expect("show reveal");
        assert!(stdout.contains("super-secret-token-1234"));
        assert!(!stdout.contains("****...1234"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn config_show_json_remains_parseable_after_redaction() {
        let _lock = super::env_test_lock().lock().expect("env lock");
        let _home = EnvVarRestore::capture("MAW_HOME");
        let _config = EnvVarRestore::capture("MAW_CONFIG_DIR");
        let root = config_seed_secret_fixture("json");
        let stdout = config_dispatch(&["show".to_owned(), "--json".to_owned()]).expect("show json");
        let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("parseable json");
        assert_eq!(parsed["federationToken"].as_str(), Some("****...1234"));
        assert_eq!(parsed["node"].as_str(), Some("bigboy-vps"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn config_show_redacts_nested_secret_keys_but_keeps_key_prefix_visible() {
        let _lock = super::env_test_lock().lock().expect("env lock");
        let _home = EnvVarRestore::capture("MAW_HOME");
        let _config = EnvVarRestore::capture("MAW_CONFIG_DIR");
        let root = config_seed_secret_fixture("nested");
        let stdout = config_dispatch(&["show".to_owned()]).expect("show");
        assert!(stdout.contains("\"apiToken\": \"****...5678\""));
        assert!(stdout.contains("\"peerKey\": \"****...4321\""));
        assert!(stdout.contains("\"pubKey\": \"****...8765\""));
        assert!(stdout.contains("\"keyPrefix\": \"maw/fleet\""));
        assert!(!stdout.contains("plugin-token-5678"));
        assert!(!stdout.contains("peer-secret-4321"));
        assert!(!stdout.contains("pubkey-secret-8765"));
        let _ = std::fs::remove_dir_all(root);
    }

    fn config_seed_secret_fixture(label: &str) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("maw-rs-config-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::env::remove_var("MAW_HOME");
        std::env::set_var("MAW_CONFIG_DIR", root.join("config"));
        std::fs::create_dir_all(root.join("config")).expect("config dir");
        std::fs::write(
            root.join("config/maw.config.50.json"),
            r#"{
  "node": "bigboy-vps",
  "port": 3456,
  "federationToken": "super-secret-token-1234",
  "namedPeers": [
    {
      "name": "peer",
      "url": "https://peer.example/api",
      "pubKey": "pubkey-secret-8765"
    }
  ],
  "plugins": {
    "foo": {
      "apiToken": "plugin-token-5678",
      "peerKey": "peer-secret-4321",
      "keyPrefix": "maw/fleet"
    }
  }
}
"#,
        )
        .expect("seed config");
        root
    }
}
