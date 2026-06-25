const DISPATCH_102: &[DispatcherEntry] = &[DispatcherEntry {
    command: "plugin",
    handler: Handler::Sync(plugin_run_command),
}];

const PLUGIN_USAGE: &str = "usage: maw plugin <ls|info|install|remove|enable|disable|init|create|build|dev> [args]\n  ls/list                  list installed plugins\n  info <name>              show manifest and resolved paths\n  install <dir> --root R   install a built plugin directory\n  remove <name> --yes      archive installed plugin directory (Nothing Deleted)\n  enable <name...>         enable plugins in the local disabled registry\n  disable <name>           disable one plugin in the local disabled registry\n  init|create <name>       create file-only JS plugin scaffold\n  build|dev                stubbed: native bun bridge pending design";
const PLUGIN_BUN_STUB: &str = "warn: plugin build/dev delegated to bun toolchain; native bun-bridge pending design (TODO #116)\n";

fn plugin_run_command(argv: &[String]) -> CliOutput {
    match plugin_parse_kind(argv).and_then(|kind| plugin_dispatch_kind(kind, &argv[1..])) {
        Ok(output) => output,
        Err(message) if message.is_empty() => plugin_ok(PLUGIN_USAGE),
        Err(message) => plugin_error(2, &message),
    }
}

fn plugin_parse_kind(argv: &[String]) -> Result<&str, String> {
    let Some(kind) = argv.first().map(String::as_str) else { return Err(String::new()); };
    if matches!(kind, "--help" | "-h" | "help") { return Err(String::new()); }
    if kind == "--" || kind.starts_with('-') { return Err("plugin: subcommand must not start with '-' or be '--'".to_owned()); }
    Ok(kind)
}

fn plugin_dispatch_kind(kind: &str, rest: &[String]) -> Result<CliOutput, String> {
    match kind {
        "ls" | "list" => Ok(run_plugin_plan(&plugin_with_subcommand("ls", rest))),
        "init" | "install" | "infer-capabilities" => Ok(run_plugin_plan(&plugin_with_subcommand(kind, rest))),
        "create" | "scaffold" => plugin_create(rest),
        "info" => plugin_info(rest),
        "enable" => plugin_enable(rest),
        "disable" => plugin_disable(rest),
        "remove" | "rm" | "uninstall" => plugin_remove(rest),
        "build" | "dev" => Ok(plugin_stub(kind)),
        other => Err(format!("plugin: unknown subcommand {other}")),
    }
}

fn plugin_with_subcommand(kind: &str, rest: &[String]) -> Vec<String> {
    let mut argv = Vec::with_capacity(rest.len() + 1);
    argv.push(kind.to_owned());
    argv.extend(rest.iter().cloned());
    argv
}

fn plugin_create(argv: &[String]) -> Result<CliOutput, String> {
    let parsed = plugin_parse_create(argv)?;
    match init_js_plugin_dir(&parsed.name, &parsed.dir) {
        Ok(summary) => Ok(CliOutput {
            code: 0,
            stdout: if parsed.plan_json { plugin_init_summary_json(&summary) } else { format!("created plugin {} {}\n", summary.name, path_string(&summary.dir)) },
            stderr: String::new(),
        }),
        Err(message) => Err(message),
    }
}

struct PluginCreateArgs { name: String, dir: std::path::PathBuf, plan_json: bool }

fn plugin_parse_create(argv: &[String]) -> Result<PluginCreateArgs, String> {
    let mut name = None;
    let mut dir = None;
    let mut plan_json = false;
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--plan-json" => plan_json = true,
            "--dir" => { dir = Some(plugin_take_path(argv, index, "--dir")?); index += 1; }
            other if !other.starts_with('-') && name.is_none() => name = Some(other.to_owned()),
            other => return Err(format!("plugin create: unknown argument {other}")),
        }
        index += 1;
    }
    let name = plugin_validate_name(&name.ok_or_else(|| "plugin create: name is required".to_owned())?)?;
    let dir = dir.unwrap_or_else(|| std::path::PathBuf::from(&name));
    Ok(PluginCreateArgs { name, dir, plan_json })
}

fn plugin_info(argv: &[String]) -> Result<CliOutput, String> {
    let parsed = plugin_parse_named_scan(argv, "info")?;
    let plugin = plugin_find_loaded(&parsed.name, &parsed.options)?;
    Ok(CliOutput { code: 0, stdout: plugin_render_info(&plugin, parsed.json), stderr: String::new() })
}

struct PluginNamedScanArgs { name: String, options: DiscoverPackagesOptions, json: bool }

fn plugin_parse_named_scan(argv: &[String], subcommand: &str) -> Result<PluginNamedScanArgs, String> {
    let mut options = plugin_discover_options();
    let mut scan_dirs = Vec::new();
    let mut name = None;
    let mut json = false;
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--json" => json = true,
            "--scan-dir" | "--root" => { scan_dirs.push(plugin_take_path(argv, index, argv[index].as_str())?); index += 1; }
            "--disabled" => { options.disabled_plugins.push(plugin_take_value(argv, index, "--disabled")?); index += 1; }
            other if !other.starts_with('-') && name.is_none() => name = Some(other.to_owned()),
            other => return Err(format!("plugin {subcommand}: unknown argument {other}")),
        }
        index += 1;
    }
    if !scan_dirs.is_empty() { options.scan_dirs = scan_dirs; }
    plugin_add_registry_disabled(&mut options);
    let name = plugin_validate_name(&name.ok_or_else(|| format!("plugin {subcommand}: name is required"))?)?;
    Ok(PluginNamedScanArgs { name, options, json })
}

fn plugin_enable(argv: &[String]) -> Result<CliOutput, String> {
    let toggle = plugin_parse_toggle(argv, true)?;
    let mut disabled = plugin_read_disabled(&toggle.root);
    let before = disabled.len();
    disabled.retain(|name| !toggle.names.contains(name));
    plugin_write_disabled(&toggle.root, &disabled)?;
    Ok(plugin_ok(&format!("enabled {} plugin{} ({} changed)", toggle.names.len(), plugin_plural(toggle.names.len()), before - disabled.len())))
}

fn plugin_disable(argv: &[String]) -> Result<CliOutput, String> {
    let toggle = plugin_parse_toggle(argv, false)?;
    let mut disabled = plugin_read_disabled(&toggle.root);
    for name in &toggle.names {
        if !disabled.contains(name) { disabled.push(name.clone()); }
    }
    disabled.sort();
    plugin_write_disabled(&toggle.root, &disabled)?;
    Ok(plugin_ok(&format!("disabled {} plugin{}", toggle.names.len(), plugin_plural(toggle.names.len()))))
}

struct PluginToggleArgs { root: std::path::PathBuf, names: Vec<String> }

fn plugin_parse_toggle(argv: &[String], many: bool) -> Result<PluginToggleArgs, String> {
    let mut root = None;
    let mut names = Vec::new();
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--root" | "--scan-dir" => { root = Some(plugin_take_path(argv, index, argv[index].as_str())?); index += 1; }
            other if !other.starts_with('-') => names.push(plugin_validate_name(other)?),
            other => return Err(format!("plugin toggle: unknown argument {other}")),
        }
        index += 1;
    }
    if names.is_empty() { return Err("plugin toggle: name is required".to_owned()); }
    if !many && names.len() != 1 { return Err("plugin disable: expected exactly one name".to_owned()); }
    Ok(PluginToggleArgs { root: root.unwrap_or_else(plugin_default_root), names })
}

fn plugin_remove(argv: &[String]) -> Result<CliOutput, String> {
    let removal = plugin_parse_remove(argv)?;
    let plugin = plugin_find_loaded(&removal.name, &removal.options)?;
    let archive = plugin_archive_dir(&removal.archive_root, &removal.name);
    std::fs::create_dir_all(&removal.archive_root).map_err(|error| format!("plugin remove: archive root failed: {error}"))?;
    std::fs::rename(&plugin.dir, &archive).map_err(|error| format!("plugin remove: archive failed: {error}"))?;
    Ok(plugin_ok(&format!("removed {} -> {}", removal.name, path_string(&archive))))
}

struct PluginRemoveArgs { name: String, options: DiscoverPackagesOptions, archive_root: std::path::PathBuf }

fn plugin_parse_remove(argv: &[String]) -> Result<PluginRemoveArgs, String> {
    let mut options = plugin_discover_options();
    let mut scan_dirs = Vec::new();
    let mut archive_root = std::env::temp_dir();
    let mut name = None;
    let mut yes = false;
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--yes" | "-y" => yes = true,
            "--scan-dir" | "--root" => { scan_dirs.push(plugin_take_path(argv, index, argv[index].as_str())?); index += 1; }
            "--archive-root" => { archive_root = plugin_take_path(argv, index, "--archive-root")?; index += 1; }
            other if !other.starts_with('-') && name.is_none() => name = Some(other.to_owned()),
            other => return Err(format!("plugin remove: unknown argument {other}")),
        }
        index += 1;
    }
    if !yes { return Err("plugin remove: refusing without --yes".to_owned()); }
    if !scan_dirs.is_empty() { options.scan_dirs = scan_dirs; }
    let name = plugin_validate_name(&name.ok_or_else(|| "plugin remove: name is required".to_owned())?)?;
    Ok(PluginRemoveArgs { name, options, archive_root })
}

fn plugin_find_loaded(name: &str, options: &DiscoverPackagesOptions) -> Result<LoadedPlugin, String> {
    discover_packages(options).plugins.into_iter().find(|plugin| plugin.manifest.name == name).ok_or_else(|| format!("plugin '{name}' not found"))
}

fn plugin_render_info(plugin: &LoadedPlugin, json: bool) -> String {
    if json { return plugin_info_json(plugin); }
    let manifest = &plugin.manifest;
    format!("{}@{}\n  tier: {}\n  kind: {}\n  disabled: {}\n  dir: {}\n  entry: {}\n  wasm: {}\n", manifest.name, manifest.version, manifest.tier.unwrap_or(PluginTier::Core).as_str(), plugin.kind.as_str(), plugin.disabled, path_string(&plugin.dir), plugin.entry_path.as_ref().map_or_else(|| "-".to_owned(), path_string), if plugin.wasm_path.as_os_str().is_empty() { "-".to_owned() } else { path_string(&plugin.wasm_path) })
}

fn plugin_info_json(plugin: &LoadedPlugin) -> String {
    let manifest = &plugin.manifest;
    format!("{{\"name\":{},\"version\":{},\"tier\":{},\"kind\":{},\"disabled\":{},\"dir\":{},\"entryPath\":{},\"wasmPath\":{}}}\n", json_string(&manifest.name), json_string(&manifest.version), json_string(manifest.tier.unwrap_or(PluginTier::Core).as_str()), json_string(plugin.kind.as_str()), plugin.disabled, json_string(&path_string(&plugin.dir)), plugin.entry_path.as_ref().map_or_else(|| "null".to_owned(), |path| json_string(&path_string(path))), if plugin.wasm_path.as_os_str().is_empty() { "null".to_owned() } else { json_string(&path_string(&plugin.wasm_path)) })
}

fn plugin_stub(kind: &str) -> CliOutput {
    CliOutput { code: 0, stdout: format!("stub: plugin {kind} requires bun toolchain; native bridge pending design\n"), stderr: PLUGIN_BUN_STUB.to_owned() }
}

fn plugin_init_summary_json(summary: &maw_plugin_manifest::PluginInitSummary) -> String {
    format!("{{\"command\":\"plugin\",\"kind\":\"create\",\"name\":{},\"dir\":{},\"manifestPath\":{},\"entryPath\":{}}}\n", json_string(&summary.name), json_string(&path_string(&summary.dir)), json_string(&path_string(&summary.manifest_path)), json_string(&path_string(&summary.entry_path)))
}

fn plugin_discover_options() -> DiscoverPackagesOptions {
    DiscoverPackagesOptions { runtime_version: "1.0.0".to_owned(), ..DiscoverPackagesOptions::default() }
}

fn plugin_add_registry_disabled(options: &mut DiscoverPackagesOptions) {
    if let Some(root) = options.scan_dirs.first() { options.disabled_plugins.extend(plugin_read_disabled(root)); }
}

fn plugin_read_disabled(root: &std::path::Path) -> Vec<String> {
    let path = plugin_disabled_path(root);
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new(); };
    serde_json::from_str::<Vec<String>>(&text).unwrap_or_default().into_iter().filter(|name| plugin_validate_name(name).is_ok()).collect()
}

fn plugin_write_disabled(root: &std::path::Path, names: &[String]) -> Result<(), String> {
    std::fs::create_dir_all(root).map_err(|error| format!("plugin toggle: root failed: {error}"))?;
    let text = serde_json::to_string_pretty(names).map_err(|error| format!("plugin toggle: serialize failed: {error}"))? + "\n";
    std::fs::write(plugin_disabled_path(root), text).map_err(|error| format!("plugin toggle: write failed: {error}"))
}

fn plugin_disabled_path(root: &std::path::Path) -> std::path::PathBuf { root.join(".disabled.json") }

fn plugin_archive_dir(root: &std::path::Path, name: &str) -> std::path::PathBuf {
    root.join(format!("maw-plugin-{name}-{}", now_iso_utc()))
}

fn plugin_default_root() -> std::path::PathBuf { maw_data_path(&real_xdg_env(), &["plugins"]) }

fn plugin_validate_name(value: &str) -> Result<String, String> {
    if value.is_empty() || value.starts_with('-') || value == "--" || value.chars().any(char::is_whitespace) { return Err(format!("plugin: invalid plugin name {value:?}")); }
    Ok(value.to_owned())
}

fn plugin_take_value(argv: &[String], index: usize, flag: &str) -> Result<String, String> {
    argv.get(index + 1).filter(|value| !value.starts_with('-')).cloned().ok_or_else(|| format!("plugin: missing {flag} value"))
}

fn plugin_take_path(argv: &[String], index: usize, flag: &str) -> Result<std::path::PathBuf, String> {
    Ok(std::path::PathBuf::from(plugin_take_value(argv, index, flag)?))
}

fn plugin_plural(count: usize) -> &'static str { if count == 1 { "" } else { "s" } }

fn plugin_ok(message: &str) -> CliOutput { CliOutput { code: 0, stdout: format!("{message}\n"), stderr: String::new() } }

fn plugin_error(code: i32, message: &str) -> CliOutput { CliOutput { code, stdout: String::new(), stderr: format!("{message}\n{PLUGIN_USAGE}\n") } }

#[cfg(test)]
mod plugin_native_tests {
    use super::{plugin_run_command, DISPATCH_102};
    use std::path::{Path, PathBuf};

    fn plugin_args(values: &[&str]) -> Vec<String> { values.iter().map(|value| (*value).to_owned()).collect() }

    fn plugin_temp_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("maw-rs-plugin-native-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("temp root");
        root
    }

    fn plugin_write(root: &Path, name: &str) {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).expect("plugin dir");
        std::fs::write(dir.join("index.ts"), "export function handle() {}\n").expect("entry");
        std::fs::write(dir.join("plugin.json"), format!(r#"{{"name":"{name}","version":"1.0.0","sdk":"*","entry":"index.ts","cli":{{"command":"{name}"}}}}"#)).expect("manifest");
    }

    #[test]
    fn plugin_dispatch_registers_scope_split_command() {
        assert_eq!(DISPATCH_102.len(), 1);
        assert_eq!(DISPATCH_102[0].command, "plugin");
    }

    #[test]
    fn plugin_management_ls_and_info_are_full_native() {
        let root = plugin_temp_root("info");
        plugin_write(&root, "alpha");
        let ls = plugin_run_command(&plugin_args(&["ls", "--scan-dir", &root.display().to_string()]));
        assert_eq!(ls.code, 0, "{}", ls.stderr);
        assert!(ls.stdout.contains("1 plugin (1 active, 0 disabled)"));
        let info = plugin_run_command(&plugin_args(&["info", "alpha", "--scan-dir", &root.display().to_string()]));
        assert_eq!(info.code, 0, "{}", info.stderr);
        assert!(info.stdout.contains("alpha@1.0.0"));
        assert!(info.stdout.contains("kind: ts"));
    }

    #[test]
    fn plugin_build_and_dev_are_bun_bridge_stubs() {
        for sub in ["build", "dev"] {
            let out = plugin_run_command(&plugin_args(&[sub]));
            assert_eq!(out.code, 0);
            assert!(out.stdout.contains("stub: plugin"));
            assert!(out.stderr.contains("TODO #116"));
        }
    }

    #[test]
    fn plugin_enable_disable_write_temp_registry() {
        let root = plugin_temp_root("toggle");
        let disable = plugin_run_command(&plugin_args(&["disable", "alpha", "--root", &root.display().to_string()]));
        assert_eq!(disable.code, 0, "{}", disable.stderr);
        let text = std::fs::read_to_string(root.join(".disabled.json")).expect("disabled registry");
        assert!(text.contains("alpha"));
        let enable = plugin_run_command(&plugin_args(&["enable", "alpha", "--root", &root.display().to_string()]));
        assert_eq!(enable.code, 0, "{}", enable.stderr);
        assert_eq!(std::fs::read_to_string(root.join(".disabled.json")).expect("registry"), "[]\n");
    }

    #[test]
    fn plugin_remove_validates_and_archives_without_delete() {
        let root = plugin_temp_root("remove");
        let archive = root.join("archive");
        plugin_write(&root, "alpha");
        let refused = plugin_run_command(&plugin_args(&["remove", "alpha", "--scan-dir", &root.display().to_string()]));
        assert_eq!(refused.code, 2);
        assert!(refused.stderr.contains("refusing without --yes"));
        let removed = plugin_run_command(&plugin_args(&["remove", "alpha", "--yes", "--scan-dir", &root.display().to_string(), "--archive-root", &archive.display().to_string()]));
        assert_eq!(removed.code, 0, "{}", removed.stderr);
        assert!(!root.join("alpha").exists());
        assert!(std::fs::read_dir(&archive).expect("archive root").next().is_some());
    }

    #[test]
    fn plugin_guards_reject_leading_dash_and_separator() {
        let bad = plugin_run_command(&plugin_args(&["--", "ls"]));
        assert_eq!(bad.code, 2);
        let bad_name = plugin_run_command(&plugin_args(&["info", "-bad"]));
        assert_eq!(bad_name.code, 2);
        assert!(bad_name.stderr.contains("unknown argument -bad"));
    }
}
