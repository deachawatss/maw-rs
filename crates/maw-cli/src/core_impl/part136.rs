const DISPATCH_136: &[DispatcherEntry] = &[DispatcherEntry {
    command: "config",
    handler: Handler::Sync(config_run_command),
}];

const CONFIG_USAGE: &str = "usage: maw config <show|sources|explain <key>|set <key> <value>> [--json]";

fn config_run_command(argv: &[String]) -> CliOutput {
    match config_dispatch(argv) {
        Ok(stdout) => CliOutput { code: 0, stdout, stderr: String::new() },
        Err(message) => CliOutput { code: 1, stdout: String::new(), stderr: format!("{message}\n") },
    }
}

fn config_dispatch(argv: &[String]) -> Result<String, String> {
    let sub = argv.first().map_or("show", String::as_str);
    let json = argv.iter().any(|arg| arg == "--json");
    match sub {
        "set" => config_set(argv, json),
        "show" => config_show(),
        _ => Err(CONFIG_USAGE.to_owned()),
    }
}

fn config_set(argv: &[String], json: bool) -> Result<String, String> {
    let key = argv.get(1).map(String::as_str).ok_or_else(|| "usage: maw config set <key> <value>".to_owned())?;
    if key.starts_with('-') { return Err("usage: maw config set <key> <value>".to_owned()); }
    let raw = argv.get(2).map(String::as_str).ok_or_else(|| "usage: maw config set <key> <value>".to_owned())?;
    let value = match key {
        "node" => serde_json::Value::String(config_validate_node(raw)?),
        "port" => serde_json::Value::Number(config_validate_port(raw)?.into()),
        _ => return Err("maw config: native set currently supports node|port".to_owned()),
    };
    let mut config = config_read_target()?;
    let object = config.as_object_mut().ok_or_else(|| "maw config: root config must be a JSON object".to_owned())?;
    object.insert(key.to_owned(), value.clone());
    config_atomic_write(&config_target_path(), &format!("{}\n", serde_json::to_string_pretty(&config).map_err(|error| format!("maw config: failed to render JSON: {error}"))?))?;
    if json {
        Ok(format!("{}\n", serde_json::to_string_pretty(&serde_json::json!({ "key": key, "value": value })).map_err(|error| format!("maw config: failed to render JSON: {error}"))?))
    } else {
        Ok(format!("{key} = {}\n", serde_json::to_string(&value).map_err(|error| format!("maw config: failed to render value: {error}"))?))
    }
}

fn config_show() -> Result<String, String> {
    let config = config_read_target()?;
    serde_json::to_string_pretty(&config)
        .map(|body| format!("{body}\n"))
        .map_err(|error| format!("maw config: failed to render JSON: {error}"))
}

fn config_read_target() -> Result<serde_json::Value, String> {
    let path = config_target_path();
    if !path.exists() { return Ok(serde_json::json!({})); }
    let raw = std::fs::read_to_string(&path).map_err(|error| format!("maw config: failed to read config: {error}"))?;
    serde_json::from_str::<serde_json::Value>(&raw).map_err(|error| format!("maw config: failed to parse config JSON: {error}"))
}

fn config_target_path() -> std::path::PathBuf {
    let env = current_xdg_env();
    let weighted = maw_config_path(&env, &["maw.config.50.json"]);
    if weighted.exists() { weighted } else { maw_config_path(&env, &["maw.config.json"]) }
}

fn config_atomic_write(path: &std::path::Path, body: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("maw config: failed to create config dir: {error}"))?;
    }
    let file_name = path.file_name().and_then(|value| value.to_str()).unwrap_or("maw.config.json");
    let tmp = path.with_file_name(format!("{file_name}.tmp"));
    std::fs::write(&tmp, body).map_err(|error| format!("maw config: failed to write temp file: {error}"))?;
    std::fs::rename(&tmp, path).map_err(|error| format!("maw config: failed to replace config: {error}"))
}

fn config_validate_node(raw: &str) -> Result<String, String> {
    let value = raw.trim();
    if value.is_empty() || value.len() > 64 || value.starts_with('-') || value.contains('/') || value.contains('\\') || value.chars().any(char::is_control) {
        return Err("maw config: invalid node name".to_owned());
    }
    if !value.chars().all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_') {
        return Err("maw config: invalid node name".to_owned());
    }
    Ok(value.to_owned())
}

fn config_validate_port(raw: &str) -> Result<u16, String> {
    if raw.starts_with('-') || raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("maw config: invalid port (expected 1-65535)".to_owned());
    }
    let port = raw.parse::<u16>().map_err(|_| "maw config: invalid port (expected 1-65535)".to_owned())?;
    if port == 0 { return Err("maw config: invalid port (expected 1-65535)".to_owned()); }
    Ok(port)
}

#[cfg(test)]
mod config_tests {
    use super::{config_dispatch, config_target_path, dispatcher_status, DispatchKind, EnvVarRestore};

    #[test]
    fn config_dispatch_registers_native() {
        assert_eq!(dispatcher_status("config"), DispatchKind::Native);
    }

    #[test]
    fn config_rejects_bad_node_and_port_before_write() {
        assert!(config_dispatch(&["set".to_owned(), "node".to_owned(), "--bad".to_owned()]).expect_err("bad node").contains("invalid node"));
        assert!(config_dispatch(&["set".to_owned(), "port".to_owned(), "0".to_owned()]).expect_err("bad port").contains("invalid port"));
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
        std::fs::write(root.join("config/maw.config.50.json"), "{\"node\":\"old\"}\n").expect("seed");
        assert_eq!(config_target_path(), root.join("config/maw.config.50.json"));
        let stdout = config_dispatch(&["set".to_owned(), "node".to_owned(), "new-node".to_owned()]).expect("set node");
        assert_eq!(stdout, "node = \"new-node\"\n");
        let body = std::fs::read_to_string(root.join("config/maw.config.50.json")).expect("config body");
        assert!(body.contains("\"node\": \"new-node\""));
        assert!(!root.join("config/maw.config.50.json.tmp").exists());
        let _ = std::fs::remove_dir_all(root);
    }
}
