const DISPATCH_58: &[DispatcherEntry] = &[DispatcherEntry {
    command: "pr",
    handler: Handler::Sync(run_pr_command),
}];

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrOptions {
    window: Option<String>,
    title: Option<String>,
    body: Option<String>,
    show_current: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrPlan {
    cwd: std::path::PathBuf,
    branch: String,
    base_repo: String,
    base_branch: String,
    title: String,
    body: String,
}

trait PrTmux {
    fn pr_current_path(&mut self) -> Result<String, String>;
    fn pr_current_session(&mut self) -> Result<String, String>;
    fn pr_window_path(&mut self, target: &str) -> Result<String, String>;
}

struct PrNativeTmux;

impl PrTmux for PrNativeTmux {
    fn pr_current_path(&mut self) -> Result<String, String> {
        pr_tmux_output(&["display-message", "-p", "#{pane_current_path}"])
    }

    fn pr_current_session(&mut self) -> Result<String, String> {
        pr_tmux_output(&["display-message", "-p", "#{session_name}"])
    }

    fn pr_window_path(&mut self, target: &str) -> Result<String, String> {
        pr_validate_tmux_target(target, "window target")?;
        pr_tmux_output(&["display-message", "-t", target, "-p", "#{pane_current_path}"])
    }
}

trait PrProcess {
    fn pr_git_branch(&mut self, cwd: &std::path::Path) -> Result<String, String>;
    fn pr_git_remote_url(&mut self, cwd: &std::path::Path, remote: &str) -> Result<String, String>;
    fn pr_gh_create(&mut self, plan: &PrPlan) -> Result<String, String>;
    fn pr_gh_view_current(&mut self, cwd: &std::path::Path) -> Result<String, String>;
}

struct PrNativeProcess;

impl PrProcess for PrNativeProcess {
    fn pr_git_branch(&mut self, cwd: &std::path::Path) -> Result<String, String> {
        pr_validate_cwd(cwd)?;
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(["branch", "--show-current"])
            .output()
            .map_err(|_| format!("not a git repo: {}", cwd.display()))?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
        } else {
            Err(format!("not a git repo: {}", cwd.display()))
        }
    }

    fn pr_git_remote_url(&mut self, cwd: &std::path::Path, remote: &str) -> Result<String, String> {
        pr_validate_cwd(cwd)?;
        pr_validate_remote_name(remote)?;
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(["remote", "get-url", remote])
            .output()
            .map_err(|error| error.to_string())?;
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned());
        }
        let code = output.status.code().unwrap_or(1);
        Err(format!("git remote get-url {remote} failed (exit {code})"))
    }

    fn pr_gh_create(&mut self, plan: &PrPlan) -> Result<String, String> {
        pr_validate_cwd(&plan.cwd)?;
        let output = std::process::Command::new("gh")
            .current_dir(&plan.cwd)
            .args([
                "pr",
                "create",
                "--repo",
                &plan.base_repo,
                "--base",
                &plan.base_branch,
                "--title",
                &plan.title,
                "--body",
                &plan.body,
            ])
            .output()
            .map_err(|error| error.to_string())?;
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned());
        }
        let code = output.status.code().unwrap_or(1);
        Err(format!("gh pr create failed (exit {code})"))
    }

    fn pr_gh_view_current(&mut self, cwd: &std::path::Path) -> Result<String, String> {
        pr_validate_cwd(cwd)?;
        let output = std::process::Command::new("gh")
            .current_dir(cwd)
            .args(["pr", "view", "--json", "number,title,url", "--jq", "#\\(.number) \\(.title) \\(.url)"])
            .output()
            .map_err(|error| error.to_string())?;
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned());
        }
        let code = output.status.code().unwrap_or(1);
        Err(format!("gh pr view failed (exit {code})"))
    }
}

fn run_pr_command(argv: &[String]) -> CliOutput {
    match pr_run(argv, &mut PrNativeTmux, &mut PrNativeProcess) {
        Ok(stdout) => CliOutput { code: 0, stdout, stderr: String::new() },
        Err(message) => CliOutput { code: 1, stdout: String::new(), stderr: format!("{message}\n") },
    }
}

fn pr_run<T: PrTmux, P: PrProcess>(argv: &[String], tmux: &mut T, process: &mut P) -> Result<String, String> {
    let options = pr_parse_args(argv)?;
    let cwd = pr_resolve_cwd(options.window.as_deref(), tmux)?;
    if options.show_current {
        return process.pr_gh_view_current(&cwd).map(|line| format!("{line}\n"));
    }
    let branch = process.pr_git_branch(&cwd)?;
    let origin_url = process.pr_git_remote_url(&cwd, "origin")?;
    let base_repo = pr_github_repo_from_remote(&origin_url)?;
    let plan = pr_build_plan(cwd, branch, base_repo, &options)?;
    let mut out = pr_render_start(&plan);
    let url = process.pr_gh_create(&plan)?;
    let _ = writeln!(out, "\x1b[32m✅\x1b[0m {url}");
    Ok(out)
}

fn pr_parse_args(argv: &[String]) -> Result<PrOptions, String> {
    let mut options = PrOptions { window: None, title: None, body: None, show_current: false };
    let mut index = 0_usize;
    while let Some(arg) = argv.get(index) {
        match arg.as_str() {
            "--help" | "-h" => return Err(pr_usage().to_owned()),
            "--show-current" => { options.show_current = true; index += 1; }
            "--title" => { options.title = Some(pr_required_value(argv, index, "--title")?); index += 2; }
            value if value.starts_with("--title=") => { options.title = Some(value["--title=".len()..].to_owned()); index += 1; }
            "--body" => { options.body = Some(pr_required_value(argv, index, "--body")?); index += 2; }
            value if value.starts_with("--body=") => { options.body = Some(value["--body=".len()..].to_owned()); index += 1; }
            value if value.starts_with('-') => return Err(format!("pr: unknown argument {value}")),
            value => { pr_set_window(&mut options, value)?; index += 1; }
        }
    }
    Ok(options)
}

fn pr_set_window(options: &mut PrOptions, value: &str) -> Result<(), String> {
    if options.window.is_some() { return Err(pr_usage().to_owned()); }
    pr_validate_window(value)?;
    options.window = Some(value.to_owned());
    Ok(())
}

fn pr_required_value(argv: &[String], index: usize, flag: &str) -> Result<String, String> {
    let Some(value) = argv.get(index + 1) else { return Err(format!("pr: {flag} requires a value")); };
    if value.starts_with('-') { return Err(format!("pr: {flag} requires a value")); }
    Ok(value.clone())
}

fn pr_resolve_cwd<T: PrTmux>(window: Option<&str>, tmux: &mut T) -> Result<std::path::PathBuf, String> {
    if std::env::var_os("TMUX").is_none() { return Err("not in a tmux session — run inside tmux".to_owned()); }
    let cwd = if let Some(window) = window {
        pr_validate_window(window)?;
        let session = tmux.pr_current_session()?.trim().to_owned();
        pr_validate_tmux_target(&session, "session")?;
        let target = format!("{session}:{window}");
        tmux.pr_window_path(&target)?
    } else {
        tmux.pr_current_path()?
    };
    let path = std::path::PathBuf::from(cwd.trim());
    pr_validate_cwd(&path)?;
    Ok(path)
}

fn pr_build_plan(
    cwd: std::path::PathBuf,
    branch: String,
    base_repo: String,
    options: &PrOptions,
) -> Result<PrPlan, String> {
    pr_validate_branch(&branch)?;
    pr_validate_base_repo(&base_repo)?;
    let title = options.title.clone().unwrap_or_else(|| pr_branch_to_title(&branch));
    pr_validate_text_arg(&title, "title")?;
    let body = options.body.clone().unwrap_or_else(|| pr_issue_body(&branch).unwrap_or_default());
    pr_validate_text_arg(&body, "body")?;
    Ok(PrPlan { cwd, branch, base_repo, base_branch: "main".to_owned(), title, body })
}

fn pr_render_start(plan: &PrPlan) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "\x1b[36m⚡\x1b[0m creating PR: \"{}\" ({})", plan.title, plan.branch);
    let _ = writeln!(out, "\x1b[36m⚡\x1b[0m target: {} ← {}", plan.base_repo, plan.base_branch);
    if let Some(issue) = pr_extract_issue_num(&plan.branch) {
        let _ = writeln!(out, "\x1b[36m⚡\x1b[0m linking to issue #{issue}");
    }
    out
}

fn pr_branch_to_title(branch: &str) -> String {
    let stripped = branch.split_once('/').map_or(branch, |(_, tail)| tail);
    let mut out = String::new();
    let mut uppercase = true;
    for ch in stripped.chars() {
        if matches!(ch, '-' | '_') { out.push(' '); uppercase = true; }
        else if uppercase { out.extend(ch.to_uppercase()); uppercase = false; }
        else { out.push(ch); }
    }
    out
}

fn pr_issue_body(branch: &str) -> Option<String> {
    pr_extract_issue_num(branch).map(|issue| format!("Closes #{issue}\nREQ: #{issue}"))
}

fn pr_extract_issue_num(branch: &str) -> Option<u64> {
    let lower = branch.to_ascii_lowercase();
    let tail = lower.split_once("issue-")?.1;
    let digits = tail.chars().take_while(char::is_ascii_digit).collect::<String>();
    (!digits.is_empty()).then(|| digits.parse().ok()).flatten()
}

fn pr_github_repo_from_remote(url: &str) -> Result<String, String> {
    let raw = url.trim().trim_end_matches('/').trim_end_matches(".git");
    let slug = raw
        .strip_prefix("https://github.com/")
        .or_else(|| raw.strip_prefix("http://github.com/"))
        .or_else(|| raw.strip_prefix("git@github.com:"))
        .or_else(|| raw.strip_prefix("ssh://git@github.com/"))
        .or_else(|| raw.strip_prefix("github.com/"))
        .ok_or_else(|| "pr: origin remote must be a GitHub URL".to_owned())?;
    let (owner, repo) = slug.split_once('/').ok_or_else(|| "pr: origin remote must use owner/repo".to_owned())?;
    pr_validate_github_segment(owner, "owner")?;
    pr_validate_github_segment(repo, "repo")?;
    let base_repo = format!("{owner}/{repo}");
    pr_validate_base_repo(&base_repo)?;
    Ok(base_repo)
}

fn pr_tmux_output(args: &[&str]) -> Result<String, String> {
    let output = std::process::Command::new("tmux")
        .args(args)
        .output()
        .map_err(|error| format!("tmux failed: {error}"))?;
    if output.status.success() { return Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned()); }
    Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
}

fn pr_usage() -> &'static str {
    "usage: maw pr [window] [--title <title>] [--body <body>] [--show-current]"
}

fn pr_validate_window(value: &str) -> Result<(), String> {
    if value.is_empty() || value.trim() != value || value.starts_with('-') || value.contains('/') {
        return Err("pr: window must be non-empty, unpadded, not start with '-', and not contain '/'".to_owned());
    }
    if value.contains("..") || value.chars().any(char::is_control) {
        return Err("pr: window contains refused characters".to_owned());
    }
    Ok(())
}

fn pr_validate_tmux_target(value: &str, name: &str) -> Result<(), String> {
    if value.is_empty() || value.trim() != value || value.starts_with('-') || value.chars().any(char::is_control) {
        return Err(format!("pr: {name} must be non-empty, unpadded, and not start with '-'"));
    }
    if value.contains("..") || value.contains('/') { return Err(format!("pr: {name} contains refused characters")); }
    Ok(())
}

fn pr_validate_cwd(path: &std::path::Path) -> Result<(), String> {
    if path.as_os_str().is_empty() || !path.is_absolute() || path.components().any(|part| matches!(part, std::path::Component::ParentDir)) {
        return Err("could not detect working directory".to_owned());
    }
    if !path.is_dir() { return Err(format!("not a git repo: {}", path.display())); }
    Ok(())
}

fn pr_validate_branch(value: &str) -> Result<(), String> {
    if value.is_empty() { return Err("detached HEAD — cannot create PR".to_owned()); }
    if value.trim() != value || value.starts_with('-') || value.contains("..") || value.chars().any(char::is_control) {
        return Err("pr: branch contains refused characters".to_owned());
    }
    Ok(())
}

fn pr_validate_base_repo(value: &str) -> Result<(), String> {
    let (owner, repo) = value.split_once('/').ok_or_else(|| "pr: base repo must use owner/repo".to_owned())?;
    pr_validate_github_segment(owner, "owner")?;
    pr_validate_github_segment(repo, "repo")?;
    if owner.eq_ignore_ascii_case("Soul-Brews-Studio") {
        return Err(format!(
            "pr: refusing to create PR against read-only upstream {value}; set origin to a fork"
        ));
    }
    Ok(())
}

fn pr_validate_github_segment(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty()
        || value.trim() != value
        || value.starts_with('-')
        || value.contains("..")
        || value.chars().any(|ch| !(ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.')))
    {
        return Err(format!("pr: GitHub {label} contains refused characters"));
    }
    Ok(())
}

fn pr_validate_remote_name(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.trim() != value
        || value.starts_with('-')
        || value.contains('/')
        || value.contains("..")
        || value.chars().any(char::is_control)
    {
        return Err("pr: remote name contains refused characters".to_owned());
    }
    Ok(())
}

fn pr_validate_text_arg(value: &str, name: &str) -> Result<(), String> {
    if value.starts_with('-') || value.chars().any(|ch| ch == '\0') {
        return Err(format!("pr: {name} contains refused characters"));
    }
    Ok(())
}

#[cfg(test)]
mod pr_tests {
    use super::*;

    #[derive(Default)]
    struct PrMockTmux { current_path: String, session: String, window_path: String }

    impl PrTmux for PrMockTmux {
        fn pr_current_path(&mut self) -> Result<String, String> { Ok(self.current_path.clone()) }
        fn pr_current_session(&mut self) -> Result<String, String> { Ok(self.session.clone()) }
        fn pr_window_path(&mut self, target: &str) -> Result<String, String> {
            assert!(!target.starts_with('-'));
            Ok(self.window_path.clone())
        }
    }

    #[derive(Default)]
    struct PrMockProcess {
        branch: String,
        origin_url: String,
        created: Vec<PrPlan>,
        viewed: Vec<String>,
    }

    impl PrProcess for PrMockProcess {
        fn pr_git_branch(&mut self, cwd: &std::path::Path) -> Result<String, String> {
            Ok(if self.branch.is_empty() { cwd.file_name().unwrap().to_string_lossy().into_owned() } else { self.branch.clone() })
        }
        fn pr_git_remote_url(&mut self, _cwd: &std::path::Path, remote: &str) -> Result<String, String> {
            assert_eq!(remote, "origin");
            Ok(if self.origin_url.is_empty() { "https://github.com/acme/demo.git".to_owned() } else { self.origin_url.clone() })
        }
        fn pr_gh_create(&mut self, plan: &PrPlan) -> Result<String, String> {
            self.created.push(plan.clone());
            Ok("https://github.com/acme/demo/pull/7".to_owned())
        }
        fn pr_gh_view_current(&mut self, cwd: &std::path::Path) -> Result<String, String> {
            self.viewed.push(cwd.display().to_string());
            Ok("#7 Demo https://github.com/acme/demo/pull/7".to_owned())
        }
    }

    fn pr_strings(values: &[&str]) -> Vec<String> { values.iter().map(|value| (*value).to_owned()).collect() }

    fn pr_temp_dir(name: &str) -> std::path::PathBuf {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("maw-rs-pr-{name}-{}-{seq}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("temp dir");
        path
    }

    #[test]
    fn pr_parse_flags_and_guard_option_injection() {
        let parsed = pr_parse_args(&pr_strings(&["codex", "--title", "Title", "--body=Body", "--show-current"])).expect("parse");
        assert_eq!(parsed.window.as_deref(), Some("codex"));
        assert_eq!(parsed.title.as_deref(), Some("Title"));
        assert_eq!(parsed.body.as_deref(), Some("Body"));
        assert!(parsed.show_current);
        assert!(pr_parse_args(&pr_strings(&["-oProxyCommand=touch-pwned"])).expect_err("guard").contains("unknown argument"));
        assert!(pr_parse_args(&pr_strings(&["--title", "-bad"])).expect_err("guard").contains("requires a value"));
        assert!(pr_validate_window("../bad").is_err());
    }

    #[test]
    fn pr_default_create_matches_maw_js_output_shape() {
        let _guard = env_test_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _restore = EnvVarRestore::capture("TMUX");
        std::env::set_var("TMUX", "/tmp/tmux,1,0");
        let repo = pr_temp_dir("create");
        let mut tmux = PrMockTmux { current_path: repo.display().to_string(), ..Default::default() };
        let mut process = PrMockProcess { branch: "agents/issue-140-pr-native".to_owned(), ..Default::default() };

        let output = pr_run(&[], &mut tmux, &mut process).expect("run");

        assert_eq!(output, include_str!("../../tests/fixtures/native-pr/create.stdout"));
        assert_eq!(process.created[0].title, "Issue 140 Pr Native");
        assert_eq!(process.created[0].body, "Closes #140\nREQ: #140");
        assert_eq!(process.created[0].base_repo, "acme/demo");
        assert_eq!(process.created[0].base_branch, "main");
    }

    #[test]
    fn pr_window_target_uses_current_session_and_show_current() {
        let _guard = env_test_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _restore = EnvVarRestore::capture("TMUX");
        std::env::set_var("TMUX", "/tmp/tmux,1,0");
        let repo = pr_temp_dir("view");
        let mut tmux = PrMockTmux { session: "13-nova".to_owned(), window_path: repo.display().to_string(), ..Default::default() };
        let mut process = PrMockProcess::default();

        let output = pr_run(&pr_strings(&["nova-codex-2", "--show-current"]), &mut tmux, &mut process).expect("view");

        assert_eq!(output, "#7 Demo https://github.com/acme/demo/pull/7\n");
        assert_eq!(process.viewed, vec![repo.display().to_string()]);
        assert!(process.created.is_empty());
    }

    #[test]
    fn pr_requires_tmux_before_env_or_process_io() {
        let _guard = env_test_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _restore = EnvVarRestore::capture("TMUX");
        std::env::remove_var("TMUX");
        let mut tmux = PrMockTmux::default();
        let mut process = PrMockProcess::default();

        let error = pr_run(&[], &mut tmux, &mut process).expect_err("tmux required");

        assert_eq!(error, "not in a tmux session — run inside tmux");
        assert!(process.created.is_empty());
    }

    #[test]
    fn pr_overrides_title_body_and_rejects_detached_head() {
        let repo = pr_temp_dir("override");
        let options = PrOptions { window: None, title: Some("Custom".to_owned()), body: Some("Body".to_owned()), show_current: false };
        let plan = pr_build_plan(repo, "feat/demo".to_owned(), "deachawatss/maw-rs".to_owned(), &options).expect("plan");
        assert_eq!(plan.title, "Custom");
        assert_eq!(plan.body, "Body");
        let error = pr_build_plan(std::path::PathBuf::from("/tmp"), String::new(), "deachawatss/maw-rs".to_owned(), &options).expect_err("detached");
        assert!(error.contains("detached HEAD"));
    }

    #[test]
    fn pr_targets_origin_fork_main_and_rejects_soul_brews_upstream() {
        let _guard = env_test_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _restore = EnvVarRestore::capture("TMUX");
        std::env::set_var("TMUX", "/tmp/tmux,1,0");
        let repo = pr_temp_dir("fork-target");
        let mut tmux = PrMockTmux { current_path: repo.display().to_string(), ..Default::default() };
        let mut process = PrMockProcess {
            branch: "agents/issue-17-help".to_owned(),
            origin_url: "git@github.com:deachawatss/maw-rs.git".to_owned(),
            ..PrMockProcess::default()
        };

        let output = pr_run(&[], &mut tmux, &mut process).expect("fork pr");

        assert!(output.contains("target: deachawatss/maw-rs ← main"), "{output}");
        assert_eq!(process.created[0].base_repo, "deachawatss/maw-rs");
        assert_eq!(process.created[0].base_branch, "main");

        let mut upstream_process = PrMockProcess {
            branch: "agents/issue-17-help".to_owned(),
            origin_url: "https://github.com/Soul-Brews-Studio/maw-rs.git".to_owned(),
            ..PrMockProcess::default()
        };
        let err = pr_run(&[], &mut tmux, &mut upstream_process).expect_err("upstream refused");
        assert!(err.contains("read-only upstream"), "{err}");
        assert!(upstream_process.created.is_empty());

        let mut mixed_case_upstream = PrMockProcess {
            branch: "agents/issue-17-help".to_owned(),
            origin_url: "git@github.com:soul-brews-studio/maw-rs.git".to_owned(),
            ..PrMockProcess::default()
        };
        let err = pr_run(&[], &mut tmux, &mut mixed_case_upstream).expect_err("upstream case refused");
        assert!(err.contains("read-only upstream"), "{err}");
        assert!(mixed_case_upstream.created.is_empty());
    }
}
