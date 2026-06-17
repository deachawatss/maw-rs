fn plugin_scaffold_usage_error(message: &str) -> CliOutput {
    CliOutput {
        code: 2,
        stdout: String::new(),
        stderr: format!(
            "{message}\nusage: maw-rs plugin-scaffold validate-name --name <name> [--plan-json]\n       maw-rs plugin-scaffold manifest --name <name> (--rust|--as) [--plan-json]\n       maw-rs plugin-scaffold constants [--plan-json]\n"
        ),
    }
}

fn run_plugin_plan(argv: &[String]) -> CliOutput {
    let action = match parse_plugin_args(argv) {
        Ok(action) => action,
        Err(message) => return plugin_usage_error(&message),
    };

    match action {
        PluginAction::Ls { options, verbose } => {
            let report = discover_packages(&options);
            CliOutput {
                code: 0,
                stdout: render_plugin_ls(&report.plugins, &report.warnings, verbose),
                stderr: String::new(),
            }
        }
    }
}

enum PluginAction {
    Ls {
        options: DiscoverPackagesOptions,
        verbose: bool,
    },
}

fn parse_plugin_args(argv: &[String]) -> Result<PluginAction, String> {
    let Some(kind) = argv.first().map(String::as_str) else {
        return Err("plugin: expected ls".to_owned());
    };
    match kind {
        "ls" | "list" => parse_plugin_ls_args(&argv[1..]),
        other => Err(format!("plugin: unknown subcommand {other}")),
    }
}

fn parse_plugin_ls_args(argv: &[String]) -> Result<PluginAction, String> {
    let mut options = DiscoverPackagesOptions::default();
    let mut scan_dirs = Vec::new();
    let mut verbose = false;
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "-v" | "--verbose" => verbose = true,
            "--scan-dir" => {
                scan_dirs.push(take_plugin_manifest_path(argv, index, "--scan-dir")?);
                index += 1;
            }
            "--disabled" => {
                options
                    .disabled_plugins
                    .push(take_plugin_manifest_value(argv, index, "--disabled")?);
                index += 1;
            }
            "--runtime-version" => {
                options.runtime_version =
                    take_plugin_manifest_value(argv, index, "--runtime-version")?;
                index += 1;
            }
            "--use-cache" => options.use_cache = true,
            other => return Err(format!("plugin ls: unknown argument {other}")),
        }
        index += 1;
    }
    if !scan_dirs.is_empty() {
        options.scan_dirs = scan_dirs;
    }

    Ok(PluginAction::Ls { options, verbose })
}

fn plugin_usage_error(message: &str) -> CliOutput {
    CliOutput {
        code: 2,
        stdout: String::new(),
        stderr: format!(
            "{message}\nusage: maw-rs plugin ls [-v|--verbose] [--scan-dir <dir>]... [--disabled <name>]... [--runtime-version <version>] [--use-cache]\n"
        ),
    }
}

fn render_plugin_ls(plugins: &[LoadedPlugin], warnings: &[String], verbose: bool) -> String {
    let mut rows = plugins.iter().map(PluginLsRow::new).collect::<Vec<_>>();
    rows.sort_by_key(|row| (plugin_tier_order(row.tier), row.name.to_owned()));

    let widths = PluginLsWidths::new(&rows);
    let mut output = String::new();
    for tier in [PluginTier::Core, PluginTier::Standard, PluginTier::Extra] {
        let tier_rows = rows
            .iter()
            .filter(|row| row.tier == tier)
            .collect::<Vec<_>>();
        if tier_rows.is_empty() {
            continue;
        }

        if !output.is_empty() {
            output.push('\n');
        }
        let _ = writeln!(output, "{} plugins", tier.as_str());
        let _ = writeln!(
            output,
            "{:<name_width$}  {:<version_width$}  {:<tier_width$}  {:<surfaces_width$}  {}",
            "name",
            "version",
            "tier",
            "surfaces",
            "dir",
            name_width = widths.name,
            version_width = widths.version,
            tier_width = widths.tier,
            surfaces_width = widths.surfaces
        );

        for row in tier_rows {
            let _ = writeln!(
                output,
                "{:<name_width$}  {:<version_width$}  {:<tier_width$}  {:<surfaces_width$}  {}",
                row.name,
                row.version,
                row.tier.as_str(),
                row.surfaces,
                row.dir,
                name_width = widths.name,
                version_width = widths.version,
                tier_width = widths.tier,
                surfaces_width = widths.surfaces
            );
            if verbose {
                if let Some(description) = row.description {
                    let _ = writeln!(output, "  description: {description}");
                }
                let _ = writeln!(output, "  kind: {}", row.kind);
                let _ = writeln!(output, "  weight: {}", row.weight);
                if let Some(command) = &row.cli_command {
                    let _ = writeln!(output, "  cli: maw {command}");
                }
                if let Some(api_path) = row.api_path {
                    let _ = writeln!(output, "  api: {api_path}");
                }
            }
        }
    }

    if output.is_empty() {
        output.push_str("no plugins found\n");
    }
    if verbose && !warnings.is_empty() {
        output.push('\n');
        output.push_str("warnings\n");
        for warning in warnings {
            let _ = writeln!(output, "  - {warning}");
        }
    }

    output
}

struct PluginLsRow<'a> {
    name: &'a str,
    version: &'a str,
    tier: PluginTier,
    surfaces: String,
    dir: String,
    kind: &'static str,
    weight: u64,
    description: Option<&'a str>,
    cli_command: Option<String>,
    api_path: Option<&'a str>,
}

impl<'a> PluginLsRow<'a> {
    fn new(plugin: &'a LoadedPlugin) -> Self {
        let manifest = &plugin.manifest;
        let cli_command = plugin_ls_cli_command(plugin);
        let api_path = manifest.api.as_ref().map(|api| api.path.as_str());
        Self {
            name: &manifest.name,
            version: &manifest.version,
            tier: manifest.tier.unwrap_or(PluginTier::Core),
            surfaces: plugin_ls_surfaces(cli_command.as_deref(), api_path),
            dir: path_string(&plugin.dir),
            kind: plugin.kind.as_str(),
            weight: manifest.weight.unwrap_or(50),
            description: manifest.description.as_deref(),
            cli_command,
            api_path,
        }
    }
}

struct PluginLsWidths {
    name: usize,
    version: usize,
    tier: usize,
    surfaces: usize,
}

impl PluginLsWidths {
    fn new(rows: &[PluginLsRow<'_>]) -> Self {
        let mut widths = Self {
            name: "name".len(),
            version: "version".len(),
            tier: "tier".len(),
            surfaces: "surfaces".len(),
        };
        for row in rows {
            widths.name = widths.name.max(row.name.len());
            widths.version = widths.version.max(row.version.len());
            widths.tier = widths.tier.max(row.tier.as_str().len());
            widths.surfaces = widths.surfaces.max(row.surfaces.len());
        }
        widths
    }
}

fn plugin_ls_surfaces(cli_command: Option<&str>, api_path: Option<&str>) -> String {
    let mut surfaces = Vec::new();
    if cli_command.is_some() {
        surfaces.push("cli");
    }
    if api_path.is_some() {
        surfaces.push("api");
    }
    if surfaces.is_empty() {
        "-".to_owned()
    } else {
        surfaces.join(",")
    }
}

fn plugin_ls_cli_command(plugin: &LoadedPlugin) -> Option<String> {
    plugin.manifest.cli.as_ref().map_or_else(
        || {
            let dispatchable = match plugin.kind {
                LoadedPluginKind::Ts => plugin.entry_path.is_some(),
                LoadedPluginKind::Wasm => !plugin.wasm_path.as_os_str().is_empty(),
            };
            dispatchable.then(|| plugin.manifest.name.clone())
        },
        |cli| Some(cli.command.clone()),
    )
}

const fn plugin_tier_order(tier: PluginTier) -> u8 {
    match tier {
        PluginTier::Core => 0,
        PluginTier::Standard => 1,
        PluginTier::Extra => 2,
    }
}

fn run_plugin_manifest_plan(argv: &[String]) -> CliOutput {
    let action = match parse_plugin_manifest_args(argv) {
        Ok(action) => action,
        Err(message) => return plugin_manifest_usage_error(&message),
    };
    match action {
        PluginManifestAction::Parse {
            plan_json,
            dir,
            json_text,
        } => match parse_manifest(&json_text, &dir) {
            Ok(manifest) => CliOutput {
                code: 0,
                stdout: if plan_json {
                    format!(
                        "{{\"command\":\"plugin-manifest\",\"kind\":\"parse\",\"dir\":{},\"manifest\":{}}}\n",
                        json_string(&path_string(&dir)),
                        render_plugin_manifest_json(&manifest)
                    )
                } else {
                    format!("{}\n", manifest.name)
                },
                stderr: String::new(),
            },
            Err(message) => plugin_manifest_usage_error(&message),
        },
        PluginManifestAction::Load { plan_json, dir } => match load_manifest_from_dir(&dir) {
            Ok(plugin) => CliOutput {
                code: 0,
                stdout: if plan_json {
                    let plugin_json = plugin
                        .as_ref()
                        .map_or_else(|| "null".to_owned(), render_loaded_plugin_json);
                    format!(
                        "{{\"command\":\"plugin-manifest\",\"kind\":\"load\",\"dir\":{},\"present\":{},\"plugin\":{plugin_json}}}\n",
                        json_string(&path_string(&dir)),
                        plugin.is_some()
                    )
                } else {
                    plugin.map_or_else(
                        || "missing\n".to_owned(),
                        |plugin| format!("{} {}\n", plugin.kind.as_str(), plugin.manifest.name),
                    )
                },
                stderr: String::new(),
            },
            Err(message) => plugin_manifest_usage_error(&message),
        },
        PluginManifestAction::Discover { plan_json, options } => {
            let report = discover_packages(&options);
            CliOutput {
                code: 0,
                stdout: if plan_json {
                    render_plugin_discover_json(&options, &report.plugins, &report.warnings)
                } else {
                    let mut names = report
                        .plugins
                        .iter()
                        .map(|plugin| plugin.manifest.name.as_str())
                        .collect::<Vec<_>>()
                        .join("\n");
                    names.push('\n');
                    names
                },
                stderr: String::new(),
            }
        }
        PluginManifestAction::ImportSymbol {
            plan_json,
            options,
            plugin,
            symbol,
            module_symbols,
        } => run_plugin_manifest_import_symbol_plan(
            plan_json,
            &options,
            &plugin,
            &symbol,
            &module_symbols,
        ),
        PluginManifestAction::Invoke {
            plan_json,
            options,
            plugin,
            source,
            args,
            fake_ts_output,
            fake_wasm_output,
        } => run_plugin_manifest_invoke_plan(
            plan_json,
            &options,
            &plugin,
            source,
            args,
            fake_ts_output,
            fake_wasm_output,
        ),
    }
}

fn run_plugin_manifest_import_symbol_plan(
    plan_json: bool,
    options: &DiscoverPackagesOptions,
    plugin: &str,
    symbol: &str,
    module_symbols: &BTreeMap<String, String>,
) -> CliOutput {
    let report = discover_packages(options);
    let mut module_path = None;
    match import_plugin_symbol(plugin, symbol, &report.plugins, |path| {
        module_path = Some(path.to_path_buf());
        Ok(module_symbols.clone())
    }) {
        Ok(value) => CliOutput {
            code: 0,
            stdout: if plan_json {
                render_plugin_import_symbol_json(
                    plugin,
                    symbol,
                    &value,
                    module_path.as_deref(),
                    &report.warnings,
                )
            } else {
                format!("{value}\n")
            },
            stderr: String::new(),
        },
        Err(message) => plugin_manifest_usage_error(&message),
    }
}

fn run_plugin_manifest_invoke_plan(
    plan_json: bool,
    options: &DiscoverPackagesOptions,
    plugin_name: &str,
    source: InvokeSource,
    args: Vec<String>,
    fake_ts_output: Option<String>,
    fake_wasm_output: Option<String>,
) -> CliOutput {
    let report = discover_packages(options);
    let Some(plugin) = report
        .plugins
        .iter()
        .find(|plugin| plugin.manifest.name == plugin_name)
    else {
        return plugin_manifest_usage_error(&format!("plugin '{plugin_name}' not found"));
    };
    if plugin.disabled {
        return plugin_manifest_usage_error(&format!("plugin '{plugin_name}' is disabled"));
    }
    let ctx = InvokeContext { source, args };
    let mut runtime = PlanInvokeRuntime::new(fake_ts_output, fake_wasm_output);
    let result = invoke_plugin(plugin, &ctx, &mut runtime);
    CliOutput {
        code: 0,
        stdout: if plan_json {
            render_plugin_invoke_json(plugin_name, &ctx, &result, &runtime, &report.warnings)
        } else if result.ok {
            result
                .output
                .map_or_else(|| "ok\n".to_owned(), |output| format!("{output}\n"))
        } else {
            format!("{}\n", result.error.unwrap_or_else(|| "error".to_owned()))
        },
        stderr: String::new(),
    }
}

struct PlanInvokeRuntime {
    ts_calls: usize,
    wasm_calls: usize,
    last_wasm_bytes_len: usize,
    ts_result: InvokeResult,
    wasm_result: InvokeResult,
}

impl PlanInvokeRuntime {
    fn new(fake_ts_output: Option<String>, fake_wasm_output: Option<String>) -> Self {
        Self {
            ts_calls: 0,
            wasm_calls: 0,
            last_wasm_bytes_len: 0,
            ts_result: fake_ts_output.map_or_else(InvokeResult::ok, InvokeResult::output),
            wasm_result: fake_wasm_output.map_or_else(InvokeResult::ok, InvokeResult::output),
        }
    }
}

impl PluginInvokeRuntime for PlanInvokeRuntime {
    fn invoke_ts(&mut self, _plugin: &LoadedPlugin, _ctx: &InvokeContext) -> InvokeResult {
        self.ts_calls += 1;
        self.ts_result.clone()
    }

    fn invoke_wasm(
        &mut self,
        _plugin: &LoadedPlugin,
        _ctx: &InvokeContext,
        wasm_bytes: &[u8],
    ) -> InvokeResult {
        self.wasm_calls += 1;
        self.last_wasm_bytes_len = wasm_bytes.len();
        self.wasm_result.clone()
    }
}

enum PluginManifestAction {
    Parse {
        plan_json: bool,
        dir: std::path::PathBuf,
        json_text: String,
    },
    Load {
        plan_json: bool,
        dir: std::path::PathBuf,
    },
    Discover {
        plan_json: bool,
        options: DiscoverPackagesOptions,
    },
    ImportSymbol {
        plan_json: bool,
        options: DiscoverPackagesOptions,
        plugin: String,
        symbol: String,
        module_symbols: BTreeMap<String, String>,
    },
    Invoke {
        plan_json: bool,
        options: DiscoverPackagesOptions,
        plugin: String,
        source: InvokeSource,
        args: Vec<String>,
        fake_ts_output: Option<String>,
        fake_wasm_output: Option<String>,
    },
}

fn parse_plugin_manifest_args(argv: &[String]) -> Result<PluginManifestAction, String> {
    let Some(kind) = argv.first().map(String::as_str) else {
        return Err("plugin-manifest: expected parse or load".to_owned());
    };
    match kind {
        "parse" => parse_plugin_manifest_parse_args(&argv[1..]),
        "load" => parse_plugin_manifest_load_args(&argv[1..]),
        "discover" => parse_plugin_manifest_discover_args(&argv[1..]),
        "import-symbol" => parse_plugin_manifest_import_symbol_args(&argv[1..]),
        "invoke" => parse_plugin_manifest_invoke_args(&argv[1..]),
        other => Err(format!("plugin-manifest: unknown subcommand {other}")),
    }
}

fn parse_plugin_manifest_parse_args(argv: &[String]) -> Result<PluginManifestAction, String> {
    let mut plan_json = false;
    let mut dir = std::path::PathBuf::from(".");
    let mut json_text = None;
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--plan-json" => plan_json = true,
            "--dir" => {
                dir = take_plugin_manifest_path(argv, index, "--dir")?;
                index += 1;
            }
            "--json" => {
                json_text = Some(take_plugin_manifest_value(argv, index, "--json")?);
                index += 1;
            }
            other => return Err(format!("plugin-manifest parse: unknown argument {other}")),
        }
        index += 1;
    }
    Ok(PluginManifestAction::Parse {
        plan_json,
        dir,
        json_text: json_text
            .ok_or_else(|| "plugin-manifest parse: --json is required".to_owned())?,
    })
}

fn parse_plugin_manifest_load_args(argv: &[String]) -> Result<PluginManifestAction, String> {
    let mut plan_json = false;
    let mut dir = std::path::PathBuf::from(".");
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--plan-json" => plan_json = true,
            "--dir" => {
                dir = take_plugin_manifest_path(argv, index, "--dir")?;
                index += 1;
            }
            other => return Err(format!("plugin-manifest load: unknown argument {other}")),
        }
        index += 1;
    }
    Ok(PluginManifestAction::Load { plan_json, dir })
}

fn parse_plugin_manifest_discover_args(argv: &[String]) -> Result<PluginManifestAction, String> {
    let (plan_json, options, _) = parse_plugin_manifest_registry_args(argv, false)?;
    Ok(PluginManifestAction::Discover { plan_json, options })
}

fn parse_plugin_manifest_import_symbol_args(
    argv: &[String],
) -> Result<PluginManifestAction, String> {
    let (plan_json, options, import) = parse_plugin_manifest_registry_args(argv, true)?;
    let import = import.expect("import parser requested import args");
    Ok(PluginManifestAction::ImportSymbol {
        plan_json,
        options,
        plugin: import.plugin,
        symbol: import.symbol,
        module_symbols: import.module_symbols,
    })
}

fn parse_plugin_manifest_invoke_args(argv: &[String]) -> Result<PluginManifestAction, String> {
    let mut plan_json = false;
    let mut scan_dirs = Vec::new();
    let mut disabled_plugins = Vec::new();
    let mut runtime_version = "1.0.0".to_owned();
    let mut use_cache = false;
    let mut plugin = None;
    let mut source = InvokeSource::Cli;
    let mut invoke_args = Vec::new();
    let mut fake_ts_output = None;
    let mut fake_wasm_output = None;
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--plan-json" => plan_json = true,
            "--scan-dir" => {
                scan_dirs.push(take_plugin_manifest_path(argv, index, "--scan-dir")?);
                index += 1;
            }
            "--disabled" => {
                disabled_plugins.push(take_plugin_manifest_value(argv, index, "--disabled")?);
                index += 1;
            }
            "--runtime-version" => {
                runtime_version = take_plugin_manifest_value(argv, index, "--runtime-version")?;
                index += 1;
            }
            "--use-cache" => use_cache = true,
            "--plugin" => {
                plugin = Some(take_plugin_manifest_value(argv, index, "--plugin")?);
                index += 1;
            }
            "--source" => {
                source = parse_plugin_manifest_invoke_source(&take_plugin_manifest_value(
                    argv, index, "--source",
                )?)?;
                index += 1;
            }
            "--arg" => {
                invoke_args.push(take_plugin_manifest_value(argv, index, "--arg")?);
                index += 1;
            }
            "--fake-ts-output" => {
                fake_ts_output = Some(take_plugin_manifest_value(argv, index, "--fake-ts-output")?);
                index += 1;
            }
            "--fake-wasm-output" => {
                fake_wasm_output = Some(take_plugin_manifest_value(
                    argv,
                    index,
                    "--fake-wasm-output",
                )?);
                index += 1;
            }
            other => return Err(format!("plugin-manifest invoke: unknown argument {other}")),
        }
        index += 1;
    }
    if scan_dirs.is_empty() {
        return Err("plugin-manifest invoke: --scan-dir is required".to_owned());
    }
    Ok(PluginManifestAction::Invoke {
        plan_json,
        options: DiscoverPackagesOptions {
            scan_dirs,
            disabled_plugins,
            runtime_version,
            use_cache,
        },
        plugin: plugin.ok_or_else(|| "plugin-manifest invoke: --plugin is required".to_owned())?,
        source,
        args: invoke_args,
        fake_ts_output,
        fake_wasm_output,
    })
}
