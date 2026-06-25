const DISPATCH_97: &[DispatcherEntry] = &[DispatcherEntry { command: "pair", handler: Handler::Sync(pair_run_command) }];

const PAIR_USAGE: &str = "usage:\n  maw pair generate [--expires <sec>] [--at <local-url>]\n  maw pair <url> <code>\n  maw pair accept <code> --at <url>";
const PAIR_ALPHABET: &str = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const PAIR_DEFAULT_EXPIRES_SEC: u64 = 120;
const PAIR_MIN_EXPIRES_SEC: u64 = 5;
const PAIR_MAX_EXPIRES_SEC: u64 = 3600;
const PAIR_BLOCKED_SUBCOMMANDS: &[&str] = &["approve", "auto-approve", "auto-pair", "pair-approve", "pair-auto", "trust"];
const PAIR_VALUE_FLAGS: &[&str] = &["--at", "--expires", "--token", "--token-ref", "--peer-token", "--federation-token"];

#[derive(Debug, Clone, PartialEq, Eq)]
enum PairAction {
    Help,
    Generate(PairGeneratePlan),
    Accept(PairAcceptPlan),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PairGeneratePlan {
    local_url: String,
    expires_sec: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PairAcceptPlan {
    remote_url: String,
    code_normalized: String,
    code_redacted: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PairConfig {
    node: String,
    port: u16,
}

trait PairHost {
    fn pair_config(&mut self) -> PairConfig;
}

struct PairSystemHost;

impl PairHost for PairSystemHost {
    fn pair_config(&mut self) -> PairConfig {
        let config = load_hey_config();
        PairConfig {
            node: config.node.unwrap_or_else(|| "local".to_owned()),
            port: 3456,
        }
    }
}

fn pair_run_command(argv: &[String]) -> CliOutput {
    let mut host = PairSystemHost;
    pair_run_command_with(argv, &mut host)
}

fn pair_run_command_with(argv: &[String], host: &mut impl PairHost) -> CliOutput {
    match pair_run(argv, host) {
        Ok(stdout) => CliOutput { code: 0, stdout, stderr: String::new() },
        Err(message) => CliOutput { code: 2, stdout: String::new(), stderr: format!("{message}\n") },
    }
}

fn pair_run(argv: &[String], host: &mut impl PairHost) -> Result<String, String> {
    pair_validate_argv(argv)?;
    match pair_parse(argv, host)? {
        PairAction::Help => Ok(pair_help()),
        PairAction::Generate(plan) => Ok(pair_render_generate(&plan)),
        PairAction::Accept(plan) => Ok(pair_render_accept(&plan, &host.pair_config())),
    }
}

fn pair_validate_argv(argv: &[String]) -> Result<(), String> {
    pair_validate_blocked_surface(argv)?;
    pair_validate_separator(argv)?;
    pair_validate_leading_dash_values(argv)?;
    pair_validate_control_free(argv)?;
    Ok(())
}

fn pair_validate_blocked_surface(argv: &[String]) -> Result<(), String> {
    let Some(first) = argv.first().map(String::as_str) else { return Ok(()); };
    if first.starts_with('-') { return Err("pair subcommand must not start with '-'".to_owned()); }
    if PAIR_BLOCKED_SUBCOMMANDS.iter().any(|blocked| blocked == &first) {
        return Err("pair: consent mutation requires explicit human pairing flow; no auto-approve surface is exposed".to_owned());
    }
    Ok(())
}

fn pair_validate_separator(argv: &[String]) -> Result<(), String> {
    if argv.iter().any(|arg| arg == "--") { return Err("pair: -- separator is not allowed".to_owned()); }
    Ok(())
}

fn pair_validate_leading_dash_values(argv: &[String]) -> Result<(), String> {
    let mut index = 0_usize;
    while index < argv.len() {
        let arg = argv[index].as_str();
        if pair_is_value_flag(arg) {
            pair_validate_flag_value(argv, index, arg)?;
            index += 2;
        } else {
            index += 1;
        }
    }
    Ok(())
}

fn pair_is_value_flag(arg: &str) -> bool {
    PAIR_VALUE_FLAGS.iter().any(|flag| flag == &arg)
}

fn pair_validate_flag_value(argv: &[String], index: usize, flag: &str) -> Result<(), String> {
    let Some(value) = argv.get(index + 1) else { return Ok(()); };
    if value == "--" || value.starts_with('-') { return Err(format!("pair: {flag} value must not start with '-'")); }
    if value.bytes().any(|byte| byte == 0 || byte.is_ascii_control()) { return Err(format!("pair: {flag} value must not contain control characters")); }
    Ok(())
}

fn pair_validate_control_free(argv: &[String]) -> Result<(), String> {
    for arg in argv {
        if arg.bytes().any(|byte| byte == 0 || byte.is_ascii_control()) { return Err("pair: arguments must not contain control characters".to_owned()); }
    }
    Ok(())
}

fn pair_parse(argv: &[String], host: &mut impl PairHost) -> Result<PairAction, String> {
    if argv.is_empty() || argv.first().is_some_and(|arg| matches!(arg.as_str(), "help" | "--help" | "-h")) { return Ok(PairAction::Help); }
    let first = argv[0].as_str();
    if first == "generate" { return pair_parse_generate(&argv[1..], host).map(PairAction::Generate); }
    if first == "accept" { return pair_parse_accept_command(&argv[1..]).map(PairAction::Accept); }
    if argv.len() >= 2 && pair_is_http_url(first) { return pair_parse_url_code(first, &argv[1]).map(PairAction::Accept); }
    Err(format!("maw pair: unexpected args (got \"{}\") — expected 'generate' or '<url> <code>'\n{PAIR_USAGE}", pair_positional_summary(argv)))
}

fn pair_parse_generate(argv: &[String], host: &mut impl PairHost) -> Result<PairGeneratePlan, String> {
    let mut expires_sec = PAIR_DEFAULT_EXPIRES_SEC;
    let mut local_url = None::<String>;
    let mut index = 0_usize;
    while index < argv.len() {
        match argv[index].as_str() {
            "--expires" => { expires_sec = pair_parse_expires(pair_next(argv, index, "--expires")?)?; index += 1; }
            value if value.starts_with("--expires=") => expires_sec = pair_parse_expires(&value["--expires=".len()..])?,
            "--at" => { local_url = Some(pair_validate_url(pair_next(argv, index, "--at")?, "--at")?); index += 1; }
            value if value.starts_with("--at=") => local_url = Some(pair_validate_url(&value["--at=".len()..], "--at")?),
            value if pair_is_token_value_flag(value) => { let _ = pair_next(argv, index, value)?; index += 1; }
            value if value.starts_with('-') => return Err(format!("pair: unknown argument {value}")),
            value => return Err(format!("pair: unexpected generate argument {value}")),
        }
        index += 1;
    }
    let config = host.pair_config();
    Ok(PairGeneratePlan { local_url: local_url.unwrap_or_else(|| format!("http://localhost:{}", config.port)), expires_sec })
}

fn pair_parse_accept_command(argv: &[String]) -> Result<PairAcceptPlan, String> {
    let Some(code) = argv.first() else { return Err("pair: accept requires <code> --at <url>".to_owned()); };
    let mut remote_url = None::<String>;
    let mut index = 1_usize;
    while index < argv.len() {
        match argv[index].as_str() {
            "--at" => { remote_url = Some(pair_validate_url(pair_next(argv, index, "--at")?, "--at")?); index += 1; }
            value if value.starts_with("--at=") => remote_url = Some(pair_validate_url(&value["--at=".len()..], "--at")?),
            value if pair_is_token_value_flag(value) => { let _ = pair_next(argv, index, value)?; index += 1; }
            value if value.starts_with('-') => return Err(format!("pair: unknown argument {value}")),
            value => return Err(format!("pair: unexpected accept argument {value}")),
        }
        index += 1;
    }
    let Some(url) = remote_url else { return Err("pair: accept requires --at <url>".to_owned()); };
    pair_parse_url_code(&url, code)
}

fn pair_parse_url_code(url: &str, raw_code: &str) -> Result<PairAcceptPlan, String> {
    let remote_url = pair_validate_url(url, "url")?;
    let code_normalized = pair_normalize_code(raw_code);
    if !pair_is_valid_code(&code_normalized) { return Err(format!("invalid code shape: {}", pair_redact_code(&code_normalized))); }
    Ok(PairAcceptPlan { remote_url, code_redacted: pair_redact_code(&code_normalized), code_normalized })
}

fn pair_next<'a>(argv: &'a [String], index: usize, flag: &str) -> Result<&'a str, String> {
    let Some(value) = argv.get(index + 1).map(String::as_str) else { return Err(format!("pair: missing value for {flag}")); };
    if value.starts_with('-') { return Err(format!("pair: missing value for {flag}")); }
    Ok(value)
}

fn pair_parse_expires(value: &str) -> Result<u64, String> {
    let parsed = value.parse::<u64>().map_err(|_| "--expires must be 5..3600 seconds".to_owned())?;
    if !(PAIR_MIN_EXPIRES_SEC..=PAIR_MAX_EXPIRES_SEC).contains(&parsed) { return Err("--expires must be 5..3600 seconds".to_owned()); }
    Ok(parsed)
}

fn pair_validate_url(raw: &str, label: &str) -> Result<String, String> {
    if raw.bytes().any(|byte| byte == 0 || byte.is_ascii_control()) || raw.starts_with('-') { return Err(format!("pair: invalid {label}")); }
    let Some((scheme, rest)) = raw.split_once("://") else { return Err(format!("invalid URL \"{raw}\"")); };
    if !matches!(scheme, "http" | "https") { return Err(format!("invalid URL \"{raw}\" (must be http:// or https://)")); }
    if rest.is_empty() || rest.starts_with('/') || rest.contains(' ') { return Err(format!("invalid URL \"{raw}\"")); }
    Ok(raw.trim_end_matches('/').to_owned())
}

fn pair_is_token_value_flag(value: &str) -> bool {
    matches!(value, "--token" | "--token-ref" | "--peer-token" | "--federation-token")
}

fn pair_is_http_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

fn pair_normalize_code(raw: &str) -> String {
    raw.chars().filter(|ch| !matches!(ch, '-' | ' ' | '\t' | '\n' | '\r')).flat_map(char::to_uppercase).collect()
}

fn pair_is_valid_code(code: &str) -> bool {
    code.len() == 6 && code.chars().all(|ch| PAIR_ALPHABET.contains(ch))
}

fn pair_redact_code(code: &str) -> String {
    let normalized = pair_normalize_code(code);
    if normalized.len() >= 3 { format!("{}-***", &normalized[..3]) } else { "***".to_owned() }
}

fn pair_pretty_code(code: &str) -> String {
    let normalized = pair_normalize_code(code);
    if normalized.len() == 6 { format!("{}-{}", &normalized[..3], &normalized[3..]) } else { normalized }
}

fn pair_positional_summary(argv: &[String]) -> String {
    argv.iter().filter(|arg| !arg.starts_with("--")).cloned().collect::<Vec<_>>().join(" ")
}

fn pair_render_generate(plan: &PairGeneratePlan) -> String {
    let ttl_ms = plan.expires_sec * 1000;
    format!(
        "🤝 pair generate plan (build-only)\n   local server: {}\n   request: POST {}/api/pair/generate {{\"ttlMs\":{ttl_ms}}}\n   waits for explicit remote accept; no auto-approve surface is exposed\n   TODO(secret): runtime code minting/status polling remains delegated to maw serve; native pair does not generate/store/echo tokens yet.\n",
        plan.local_url, plan.local_url
    )
}

fn pair_render_accept(plan: &PairAcceptPlan, config: &PairConfig) -> String {
    let warning = pair_plain_http_warning(&plan.remote_url);
    let local_url = format!("http://localhost:{}", config.port);
    format!(
        "🤝 pair accept plan (build-only)\n   remote: {}/api/pair/{}\n   code: {}\n   body: {{\"node\":{},\"url\":{}}}\n{}   human consent required; no auto-approve surface is exposed\n   TODO(secret): handshake POST, federation token receipt, and peers.json write are intentionally stubbed for Bigboy+TK gate.\n",
        plan.remote_url,
        plan.code_normalized,
        pair_pretty_code(&plan.code_normalized),
        json_string(&config.node),
        json_string(&local_url),
        warning
    )
}

fn pair_plain_http_warning(url: &str) -> String {
    if !url.starts_with("http://") { return String::new(); }
    let host = url.trim_start_matches("http://").split(['/', ':']).next().unwrap_or_default();
    if matches!(host, "localhost" | "127.0.0.1" | "::1") { String::new() } else { "   ⚠ pairing over plain HTTP — TLS recommended for cross-network pairing\n".to_owned() }
}

fn pair_help() -> String {
    [
        PAIR_USAGE,
        "",
        "example: B: `maw pair generate` → prints W4K-7F3; A: `maw pair http://b:5002 W4K-7F3`",
        "human consent is required; no auto approval or token-writing surface is exposed.",
        "build-only native port: token minting/handshake/storage are stubbed pending Bigboy+TK secret gate.",
    ].join("\n") + "\n"
}

#[cfg(test)]
mod pair_tests {
    use super::*;

    struct PairFakeHost;

    impl PairHost for PairFakeHost {
        fn pair_config(&mut self) -> PairConfig { PairConfig { node: "fake-node".to_owned(), port: 5002 } }
    }

    fn pair_args(values: &[&str]) -> Vec<String> { values.iter().map(|value| (*value).to_owned()).collect() }

    fn pair_output(values: &[&str]) -> CliOutput {
        let mut host = PairFakeHost;
        pair_run_command_with(&pair_args(values), &mut host)
    }

    #[test]
    fn pair_dispatch_registers_single_native_command() {
        assert_eq!(DISPATCH_97.len(), 1);
        assert_eq!(DISPATCH_97[0].command, "pair");
        assert_eq!(dispatcher_status("pair"), DispatchKind::Native);
    }

    #[test]
    fn pair_generate_is_plan_only_and_does_not_echo_fake_token() {
        let output = pair_output(&["generate", "--expires", "60", "--at", "http://localhost:5002", "--token", "fake-test-token"]);
        assert_eq!(output.code, 0, "{}", output.stderr);
        assert!(output.stdout.contains("pair generate plan"));
        assert!(output.stdout.contains("ttlMs\":60000"));
        assert!(output.stdout.contains("TODO(secret)"));
        assert!(!output.stdout.contains("fake-test-token"));
        assert!(!output.stderr.contains("fake-test-token"));
    }

    #[test]
    fn pair_accept_url_code_is_plan_only_and_redacts_flow() {
        let output = pair_output(&["http://peer.example:5002", "W4K-7F3"]);
        assert_eq!(output.code, 0, "{}", output.stderr);
        assert!(output.stdout.contains("pair accept plan"));
        assert!(output.stdout.contains("W4K7F3"));
        assert!(output.stdout.contains("fake-node"));
        assert!(output.stdout.contains("plain HTTP"));
    }

    #[test]
    fn pair_accept_subcommand_supports_at_url() {
        let output = pair_output(&["accept", "W4K-7F3", "--at", "https://peer.example"]);
        assert_eq!(output.code, 0, "{}", output.stderr);
        assert!(output.stdout.contains("https://peer.example/api/pair/W4K7F3"));
    }

    #[test]
    fn pair_refuses_auto_approve_surface_before_secret_values() {
        for blocked in PAIR_BLOCKED_SUBCOMMANDS {
            let output = pair_output(&[blocked, "--token", "fake-test-token"]);
            assert_eq!(output.code, 2, "blocked {blocked}");
            assert!(output.stderr.contains("no auto-approve"));
            assert!(!output.stderr.contains("fake-test-token"));
        }
    }

    #[test]
    fn pair_guards_separator_and_leading_dash_values_without_secret_echo() {
        let sep = pair_output(&["generate", "--"]);
        assert_eq!(sep.code, 2);
        assert!(sep.stderr.contains("separator"));
        let bad = pair_output(&["generate", "--token", "-secret-token"]);
        assert_eq!(bad.code, 2);
        assert!(bad.stderr.contains("--token value must not start"));
        assert!(!bad.stderr.contains("secret-token"));
    }

    #[test]
    fn pair_validates_expires_url_and_code_shape() {
        assert!(pair_output(&["generate", "--expires", "4"]).stderr.contains("5..3600"));
        assert!(pair_output(&["generate", "--at", "ftp://peer"]).stderr.contains("must be http"));
        assert!(pair_output(&["https://peer", "BAD000"]).stderr.contains("invalid code shape"));
    }

    #[test]
    fn pair_help_has_no_auto_approve_surface() {
        let output = pair_output(&[]);
        assert_eq!(output.code, 0);
        assert!(!output.stdout.contains("auto-approve"));
        assert!(output.stdout.contains("human"));
    }
}
