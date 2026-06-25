const DISPATCH_120: &[DispatcherEntry] = &[DispatcherEntry { command: "channel", handler: Handler::Sync(channel_run_command) }];

const CHANNEL_HELP: &str = "usage: maw channel <subcommand> [args]\n\nsubcommands:\n  ls [oracle] [--json] [-v] list channels (all or for specific oracle)\n  add <oracle> <plugin>    add channel plugin to oracle\n  rm <oracle> <plugin>     remove channel plugin from oracle\n  providers                list available channel providers\n  setup <oracle>           interactive channel setup wizard\n  test <oracle>            test channel configuration\n  migrate --to-repo [...]  copy global ~/.claude/channels/<oracle>/config.json\n                           into each oracle's <repo>/.claude/channel.json\n                           ([oracle...] empty = all; --dry-run / --remove-global)\n\nshorthand: discord → plugin:discord@claude-plugins-official\ngithub: prefix → delegates to setup wizard";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
struct ChannelConfig {
    plugins: Vec<ChannelPlugin>,
    token_source: Option<String>,
    #[serde(rename = "permissionMode")]
    permission_mode: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
struct ChannelPlugin {
    id: String,
    env: Option<std::collections::BTreeMap<String, String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChannelProvider {
    short_name: String,
    plugin_id: String,
    kind: &'static str,
}

fn channel_run_command(argv: &[String]) -> CliOutput {
    match channel_run(argv) {
        Ok(stdout) | Err((0, stdout)) => CliOutput { code: 0, stdout, stderr: String::new() },
        Err((code, message)) => CliOutput { code, stdout: String::new(), stderr: format!("{message}\n") },
    }
}

fn channel_run(argv: &[String]) -> Result<String, (i32, String)> {
    let sub = argv.first().map(|value| value.to_ascii_lowercase());
    match sub.as_deref() {
        Some("help" | "--help" | "-h") => Ok(format!("{CHANNEL_HELP}\n")),
        Some("ls" | "list") | None => channel_ls(&argv[1.min(argv.len())..]),
        Some("providers") => channel_providers(&argv[1..]),
        Some("test") => channel_test(&argv[1..]),
        Some("add" | "rm" | "remove" | "setup" | "migrate") => Err((1, format!("channel: subcommand '{}' is not part of read-only native slice", sub.unwrap_or_default()))),
        Some(_) => Ok(channel_short_usage()),
    }
}

fn channel_short_usage() -> String {
    "usage: maw channel <add|rm|ls|providers|setup|test|migrate> [oracle] [plugin]\n\n  maw channel providers                          list available providers\n  maw channel setup hermes-discord discord       interactive wizard\n  maw channel setup myoracle github:org/repo     git channel wizard\n  maw channel add hermes-discord discord         quick register\n  maw channel add myoracle github:org/repo       git channel\n  maw channel rm hermes-discord discord          remove channel\n  maw channel ls                                 list all\n  maw channel test hermes-discord                verify connectivity\n  maw channel migrate --to-repo [oracle...]      global → repo (#1195)\n\n  maw wake <oracle> auto-injects --channels when config exists\n".to_owned()
}

fn channel_ls(argv: &[String]) -> Result<String, (i32, String)> {
    let (target, json, verbose) = channel_parse_ls(argv)?;
    if json { return Ok(channel_ls_json(target.as_deref())); }
    if let Some(target) = target { return Ok(channel_ls_one(&target, verbose)); }
    Ok(channel_ls_all(verbose))
}

fn channel_parse_ls(argv: &[String]) -> Result<(Option<String>, bool, bool), (i32, String)> {
    let mut target = None;
    let mut json = false;
    let mut verbose = false;
    for arg in argv {
        match arg.as_str() {
            "--json" => json = true,
            "--verbose" | "-v" => verbose = true,
            "--" => return Err((2, "channel: -- separator is not supported".to_owned())),
            value if value.starts_with('-') => return Err((2, format!("channel: unknown ls flag {value}"))),
            value => {
                if target.is_some() { return Err((2, "channel ls accepts at most one oracle".to_owned())); }
                target = Some(channel_validate_name("oracle", value)?);
            }
        }
    }
    Ok((target, json, verbose))
}

fn channel_providers(argv: &[String]) -> Result<String, (i32, String)> {
    use std::fmt::Write as _;

    channel_reject_extra_args("providers", argv)?;
    let providers = channel_get_providers();
    let mut stdout = format!("  \x1b[36;1mChannel Providers\x1b[0m ({} available)\n\n", providers.len());
    stdout.push_str("  Provider        Type       Plugin ID                                     Status\n");
    stdout.push_str("  ─────────────── ────────── ───────────────────────────────────────────── ──────────\n");
    for provider in providers {
        let status = if channel_is_plugin_installed(&provider.short_name) { "\x1b[32m✓ installed\x1b[0m" } else { "\x1b[90mnot installed\x1b[0m" };
        let _ = writeln!(stdout, "  {:<15} {:<10} {:<45} {status}", provider.short_name, provider.kind, provider.plugin_id);
    }
    stdout.push_str("\n  Install: \x1b[36m/plugin install <provider>@claude-plugins-official\x1b[0m\n");
    stdout.push_str("  Custom:  \x1b[36mmaw channel add <oracle> server:<name>\x1b[0m (for .mcp.json servers)\n");
    Ok(stdout)
}

fn channel_test(argv: &[String]) -> Result<String, (i32, String)> {
    let target = channel_parse_test(argv)?;
    let Some(config) = channel_load_oracle_config(&target) else {
        return Err((1, format!("  \x1b[31m✗\x1b[0m no channels for {target}")));
    };
    if config.plugins.is_empty() { return Err((1, format!("  \x1b[31m✗\x1b[0m no channels for {target}"))); }
    let env = channel_effective_env(&config);
    let mut stdout = format!("  \x1b[36;1mChannel Test: {target}\x1b[0m\n\n");
    for plugin in &config.plugins {
        stdout.push_str("  ");
        stdout.push_str(&plugin.id);
        stdout.push('\n');
        for check in channel_checks(plugin, &config, &env) {
            stdout.push_str("    ");
            stdout.push_str(&check);
            stdout.push('\n');
        }
    }
    Ok(stdout)
}

fn channel_parse_test(argv: &[String]) -> Result<String, (i32, String)> {
    match argv {
        [] => Err((1, "  usage: maw channel test <oracle>".to_owned())),
        [target] => channel_validate_name("oracle", target),
        _ => Err((2, "channel test accepts exactly one oracle".to_owned())),
    }
}

fn channel_reject_extra_args(subcommand: &str, argv: &[String]) -> Result<(), (i32, String)> {
    if argv.iter().any(|arg| arg == "--") { return Err((2, "channel: -- separator is not supported".to_owned())); }
    if let Some(arg) = argv.first() { return Err((2, format!("channel {subcommand}: unexpected argument {arg}"))); }
    Ok(())
}

fn channel_validate_name(label: &str, value: &str) -> Result<String, (i32, String)> {
    if value.is_empty()
        || value.trim() != value
        || value.starts_with('-')
        || value == "."
        || value == ".."
        || value.contains("..")
        || value.contains('/')
        || value.contains('\\')
        || value.chars().any(char::is_control)
    {
        return Err((2, format!("channel: invalid {label}")));
    }
    if !value.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.')) {
        return Err((2, format!("channel: invalid {label}")));
    }
    Ok(value.to_owned())
}

fn channel_ls_json(target: Option<&str>) -> String {
    if let Some(target) = target {
        let config = channel_redacted_config(channel_load_oracle_config(target).unwrap_or_default());
        let mut value = serde_json::to_value(config).expect("channel config json");
        if let serde_json::Value::Object(map) = &mut value { map.insert("oracle".to_owned(), serde_json::json!(target)); }
        return format!("{}\n", serde_json::to_string_pretty(&value).expect("json"));
    }
    let oracles = channel_list_all_configs()
        .into_iter()
        .map(|(oracle, config)| serde_json::json!({ "oracle": oracle, "plugins": channel_redacted_config(config).plugins }))
        .collect::<Vec<_>>();
    format!("{}\n", serde_json::to_string_pretty(&serde_json::json!({ "oracles": oracles })).expect("json"))
}

fn channel_redacted_config(mut config: ChannelConfig) -> ChannelConfig {
    for plugin in &mut config.plugins {
        if let Some(env) = &mut plugin.env {
            for (key, value) in env.iter_mut() {
                if channel_is_secret_key(key) { "<redacted>".clone_into(value); }
            }
        }
    }
    if let Some(token_source) = &config.token_source {
        config.token_source = Some(channel_display_token_source(token_source));
    }
    config
}

fn channel_ls_one(target: &str, verbose: bool) -> String {
    let Some(config) = channel_load_oracle_config(target) else { return format!("  \x1b[90mno channels for {target}\x1b[0m\n"); };
    if config.plugins.is_empty() { return format!("  \x1b[90mno channels for {target}\x1b[0m\n"); }
    let mut stdout = format!("  \x1b[36;1m{target}\x1b[0m\n");
    for plugin in &config.plugins {
        stdout.push_str("    ");
        stdout.push_str(&plugin.id);
        stdout.push('\n');
        channel_push_plugin_env(&mut stdout, plugin, 6);
    }
    channel_push_token_source(&mut stdout, &config, 4);
    if verbose { channel_push_permission(&mut stdout, &config, 4); }
    stdout
}

fn channel_ls_all(verbose: bool) -> String {
    use std::fmt::Write as _;

    let all = channel_list_all_configs();
    if all.is_empty() {
        return "  \x1b[90mno oracles have channels configured\x1b[0m\n  add one: \x1b[36mmaw channel add <oracle> discord\x1b[0m\n".to_owned();
    }
    let mut stdout = format!("  \x1b[36;1mOracle{}Channel\x1b[0m\n", " ".repeat(24));
    let _ = writeln!(stdout, "  {}  {}", "─".repeat(30), "─".repeat(45));
    for (oracle, config) in &all {
        for plugin in &config.plugins {
            let _ = writeln!(stdout, "  {oracle:<30}  {}", plugin.id);
            if verbose {
                channel_push_plugin_env(&mut stdout, plugin, 32);
                channel_push_permission(&mut stdout, config, 32);
                channel_push_token_source(&mut stdout, config, 32);
            }
        }
    }
    let _ = writeln!(stdout, "\n  {} oracle(s) with channels", all.len());
    stdout
}

fn channel_push_plugin_env(stdout: &mut String, plugin: &ChannelPlugin, indent: usize) {
    use std::fmt::Write as _;

    if let Some(env) = &plugin.env {
        for (key, value) in env {
            let value = channel_display_env_value(key, value);
            let _ = writeln!(stdout, "{}\x1b[90m{key}={value}\x1b[0m", " ".repeat(indent));
        }
    }
}

fn channel_push_permission(stdout: &mut String, config: &ChannelConfig, indent: usize) {
    use std::fmt::Write as _;

    if let Some(mode) = &config.permission_mode {
        let _ = writeln!(stdout, "{}\x1b[90mpermissionMode: {mode}\x1b[0m", " ".repeat(indent));
    }
}

fn channel_push_token_source(stdout: &mut String, config: &ChannelConfig, indent: usize) {
    use std::fmt::Write as _;

    if let Some(token_source) = &config.token_source {
        let token_source = channel_display_token_source(token_source);
        let _ = writeln!(stdout, "{}\x1b[90mtoken: {token_source}\x1b[0m", " ".repeat(indent));
    }
}

fn channel_display_env_value(key: &str, value: &str) -> String {
    if channel_is_secret_key(key) { "<redacted>".to_owned() } else { value.to_owned() }
}

fn channel_display_token_source(value: &str) -> String {
    if matches!(value.split_once(':'), Some(("pass" | "env" | "keychain", _))) { value.to_owned() } else { "<redacted>".to_owned() }
}

fn channel_is_secret_key(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    ["TOKEN", "SECRET", "PASSWORD", "PASS", "PRIVATE_KEY"].iter().any(|needle| upper.contains(needle))
}

fn channel_get_providers() -> Vec<ChannelProvider> {
    let mut providers = vec![
        channel_provider("discord", "plugin:discord@claude-plugins-official", "chat"),
        channel_provider("telegram", "plugin:telegram@claude-plugins-official", "chat"),
        channel_provider("imessage", "plugin:imessage@claude-plugins-official", "chat"),
        channel_provider("fakechat", "plugin:fakechat@claude-plugins-official", "chat"),
    ];
    providers.extend(channel_custom_providers());
    providers
}

fn channel_provider(short_name: &str, plugin_id: &str, kind: &'static str) -> ChannelProvider {
    ChannelProvider { short_name: short_name.to_owned(), plugin_id: plugin_id.to_owned(), kind }
}

fn channel_custom_providers() -> Vec<ChannelProvider> {
    let mut providers = Vec::new();
    for path in [std::env::current_dir().ok().map(|cwd| cwd.join(".mcp.json")), Some(channel_home().join(".claude.json"))].into_iter().flatten() {
        let Some(json) = channel_read_json(&path) else { continue; };
        let Some(servers) = json.get("mcpServers").and_then(serde_json::Value::as_object) else { continue; };
        for name in servers.keys() {
            if channel_validate_name("server", name).is_ok() { providers.push(channel_provider(name, &format!("server:{name}"), "custom")); }
        }
    }
    providers
}

fn channel_is_plugin_installed(short_name: &str) -> bool {
    channel_home().join(".claude/plugins/cache/claude-plugins-official").join(short_name).exists()
}

fn channel_checks(plugin: &ChannelPlugin, config: &ChannelConfig, env: &std::collections::BTreeMap<String, String>) -> Vec<String> {
    let mut checks = Vec::new();
    if plugin.id.starts_with("plugin:") {
        let name = plugin.id.split(':').nth(1).and_then(|value| value.split('@').next()).unwrap_or_default();
        if channel_is_plugin_installed(name) { checks.push("\x1b[32m✓ plugin installed\x1b[0m".to_owned()); } else { checks.push("\x1b[31m✗ plugin not installed\x1b[0m".to_owned()); }
    }
    if let Some(dir) = env.get("DISCORD_STATE_DIR").or_else(|| plugin.env.as_ref().and_then(|map| map.get("DISCORD_STATE_DIR"))) {
        if std::path::Path::new(dir).exists() { checks.push("\x1b[32m✓ state dir exists\x1b[0m".to_owned()); } else { checks.push(format!("\x1b[31m✗ state dir missing: {dir}\x1b[0m")); }
    }
    if env.contains_key("DISCORD_BOT_TOKEN") || env.contains_key("TELEGRAM_BOT_TOKEN") { checks.push("\x1b[32m✓ token available\x1b[0m".to_owned()); } else if let Some(token_source) = &config.token_source { checks.push(format!("\x1b[32m✓ token source: {token_source}\x1b[0m")); } else { checks.push("\x1b[33m⚠ no token configured\x1b[0m".to_owned()); }
    checks
}

fn channel_effective_env(config: &ChannelConfig) -> std::collections::BTreeMap<String, String> {
    let mut env = std::collections::BTreeMap::new();
    for plugin in &config.plugins {
        if let Some(plugin_env) = &plugin.env { env.extend(plugin_env.clone()); }
    }
    let home = channel_home();
    for value in env.values_mut() {
        if let Some(stripped) = value.strip_prefix("~/") { *value = home.join(stripped).to_string_lossy().into_owned(); }
    }
    env
}

fn channel_list_all_configs() -> Vec<(String, ChannelConfig)> {
    let base = channel_channels_base();
    let Ok(entries) = std::fs::read_dir(base) else { return Vec::new(); };
    let mut configs = Vec::new();
    for entry in entries.flatten() {
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) { continue; }
        let oracle = entry.file_name().to_string_lossy().into_owned();
        if channel_validate_name("oracle", &oracle).is_err() { continue; }
        if let Some(config) = channel_load_oracle_config(&oracle) {
            if !config.plugins.is_empty() { configs.push((oracle, config)); }
        }
    }
    configs.sort_by(|left, right| left.0.cmp(&right.0));
    configs
}

fn channel_load_oracle_config(oracle: &str) -> Option<ChannelConfig> {
    let path = channel_channels_base().join(oracle).join("config.json");
    channel_read_json(&path).and_then(|value| serde_json::from_value(value).ok())
}

fn channel_read_json(path: &std::path::Path) -> Option<serde_json::Value> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn channel_channels_base() -> std::path::PathBuf { channel_home().join(".claude").join("channels") }

fn channel_home() -> std::path::PathBuf {
    std::env::var_os("HOME").map_or_else(|| std::path::PathBuf::from("."), std::path::PathBuf::from)
}
