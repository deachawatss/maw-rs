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

#[derive(Debug, Clone, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct DeliveryEvidence {
    version: u64,
    issue: u64,
    mode: String,
    #[serde(default)]
    risk_tags: Vec<String>,
    engine: String,
    spec: Option<String>,
    verification: DeliveryVerification,
}

#[derive(Debug, Clone, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct DeliveryVerification {
    commands: Vec<DeliveryCommand>,
    live_evidence: String,
    #[serde(default)]
    artifacts: Vec<String>,
    #[serde(default)]
    open_risks: Vec<String>,
}

#[derive(Debug, Clone, serde::Deserialize, PartialEq, Eq)]
struct DeliveryCommand {
    command: String,
    result: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct PrReviewRequest {
    version: u64,
    pr_url: String,
    pr_number: u64,
    repo: String,
    branch: String,
    status: String,
    notified: bool,
    notified_at: Option<String>,
    notifier: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum PrL1Notification {
    #[default]
    Hey,
    LegacyPaneFallback,
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
    fn pr_enqueue_review(&mut self, request: &PrReviewRequest) -> Result<(), String>;
    fn pr_notify_l1(&mut self, cwd: &std::path::Path, message: &str) -> Result<PrL1Notification, String>;
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

    fn pr_enqueue_review(&mut self, request: &PrReviewRequest) -> Result<(), String> {
        pr_enqueue_global_review(request)
    }

    fn pr_notify_l1(&mut self, cwd: &std::path::Path, message: &str) -> Result<PrL1Notification, String> {
        if let Some(oracle) = pr_l1_oracle(cwd) {
            pr_notify_l1_with_hey(&oracle, message)?;
            return Ok(PrL1Notification::Hey);
        }
        let pane = pr_l1_pane(cwd)
            .ok_or_else(|| "pr: L1 oracle unavailable and legacy pane unavailable; queued for hook recovery".to_owned())?;
        let literal = std::process::Command::new("tmux")
            .args(["send-keys", "-t", &pane, "-l", message])
            .output()
            .map_err(|error| format!("pr: notify L1: {error}"))?;
        if !literal.status.success() {
            return Err("pr: notify L1 literal send failed; queued for hook recovery".to_owned());
        }
        let enter = std::process::Command::new("tmux")
            .args(["send-keys", "-t", &pane, "Enter"])
            .output()
            .map_err(|error| format!("pr: notify L1 enter: {error}"))?;
        if enter.status.success() {
            Ok(PrL1Notification::LegacyPaneFallback)
        } else {
            Err("pr: notify L1 enter failed; queued for hook recovery".to_owned())
        }
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
    let pr_number = pr_extract_pr_number(&url)
        .ok_or_else(|| format!("pr: could not determine PR number from gh response: {url}"))?;
    let mut request = PrReviewRequest {
        version: 1,
        pr_url: url.clone(),
        pr_number,
        repo: plan.base_repo.clone(),
        branch: plan.branch.clone(),
        status: "pending".to_owned(),
        notified: false,
        notified_at: None,
        notifier: None,
    };
    pr_write_review_request(&plan.cwd, &request)?;
    process.pr_enqueue_review(&request)?;
    let issue = pr_extract_issue_num(&plan.branch).unwrap_or_default();
    let message = format!("[codex] PR #{pr_number} ready for issue #{issue}. {url}");
    match process.pr_notify_l1(&plan.cwd, &message) {
        Ok(notification) => {
            if notification == PrL1Notification::LegacyPaneFallback {
                let _ = writeln!(out, "\x1b[33m⚠\x1b[0m pr: L1 oracle metadata unavailable; used legacy .maw/l1-pane fallback");
            }
            request.notified = true;
            "notified".clone_into(&mut request.status);
            request.notified_at = Some(pr_now_epoch());
            request.notifier = Some("maw-pr".to_owned());
            pr_write_review_request(&plan.cwd, &request)?;
            process.pr_enqueue_review(&request)?;
            std::fs::write(plan.cwd.join(".maw/delivery-notified"), format!("{url}\n"))
                .map_err(|error| format!("pr: write delivery-notified: {error}"))?;
            let _ = writeln!(out, "\x1b[32m✅\x1b[0m L1 notified");
        }
        Err(error) => {
            let _ = writeln!(out, "\x1b[33m⚠\x1b[0m {error}");
        }
    }
    Ok(out)
}

fn pr_l1_oracle(cwd: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(cwd.join(".maw/l1-oracle"))
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| pr_valid_oracle_name(value))
}

fn pr_l1_pane(cwd: &std::path::Path) -> Option<String> {
    std::env::var("MAW_L1_PANE")
        .ok()
        .or_else(|| std::fs::read_to_string(cwd.join(".maw/l1-pane")).ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| pr_valid_pane_id(value))
}

fn pr_valid_oracle_name(value: &str) -> bool {
    !value.is_empty()
        && value.trim() == value
        && !value.starts_with('-')
        && value.chars().all(|ch| !ch.is_control() && !ch.is_whitespace())
}

fn pr_notify_l1_with_hey(oracle: &str, message: &str) -> Result<(), String> {
    let oracle = oracle.to_owned();
    let message = message.to_owned();
    let worker = std::thread::Builder::new()
        .name("maw-pr-l1-notify".to_owned())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| format!("pr: start L1 notify runtime: {error}"))?;
            Ok::<CliOutput, String>(runtime.block_on(run_hey_in_process(&oracle, &message, false)))
        })
        .map_err(|error| format!("pr: start L1 notify worker: {error}"))?;
    let output = worker
        .join()
        .map_err(|_| "pr: L1 notify worker panicked".to_owned())??;
    if output.code == 0 {
        return Ok(());
    }
    let detail = if output.stderr.trim().is_empty() {
        output.stdout.trim()
    } else {
        output.stderr.trim()
    };
    Err(format!("pr: notify L1 via maw hey failed: {detail}"))
}

fn pr_write_review_request(cwd: &std::path::Path, request: &PrReviewRequest) -> Result<(), String> {
    let dir = cwd.join(".maw");
    std::fs::create_dir_all(&dir).map_err(|error| format!("pr: create {}: {error}", dir.display()))?;
    let path = dir.join("l1-review-request.json");
    let tmp = dir.join(format!(".l1-review-request.{}.tmp", std::process::id()));
    let body = serde_json::to_string_pretty(request)
        .map_err(|error| format!("pr: render review request: {error}"))?
        + "\n";
    std::fs::write(&tmp, body).map_err(|error| format!("pr: write {}: {error}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|error| format!("pr: replace {}: {error}", path.display()))
}

fn pr_enqueue_global_review(request: &PrReviewRequest) -> Result<(), String> {
    let root = std::env::var_os("MAW_STATE_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".maw")))
        .ok_or_else(|| "pr: HOME/MAW_STATE_DIR unavailable for review queue".to_owned())?;
    std::fs::create_dir_all(&root).map_err(|error| format!("pr: create review queue dir: {error}"))?;
    let _lock = PrQueueLock::acquire(&root)?;
    let path = root.join("pr-queue.jsonl");
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let row = serde_json::to_string(request).map_err(|error| format!("pr: render queue row: {error}"))?;
    let mut replaced = false;
    let mut lines = existing
        .lines()
        .map(|line| {
            let matches = serde_json::from_str::<PrReviewRequest>(line).is_ok_and(|entry| {
                entry.repo == request.repo && entry.pr_url == request.pr_url
            });
            if matches {
                replaced = true;
                row.clone()
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>();
    if !replaced {
        lines.push(row);
    }
    let body = lines.join("\n") + "\n";
    let tmp = root.join(format!(".pr-queue.{}.tmp", std::process::id()));
    std::fs::write(&tmp, body).map_err(|error| format!("pr: write review queue: {error}"))?;
    std::fs::rename(&tmp, &path).map_err(|error| format!("pr: replace review queue: {error}"))
}

struct PrQueueLock {
    path: std::path::PathBuf,
}

impl PrQueueLock {
    fn acquire(root: &std::path::Path) -> Result<Self, String> {
        let path = root.join(".pr-queue.lock");
        for _ in 0..100 {
            match std::fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = std::fs::metadata(&path)
                        .and_then(|metadata| metadata.modified())
                        .ok()
                        .and_then(|modified| modified.elapsed().ok())
                        .is_some_and(|age| age > std::time::Duration::from_mins(1));
                    if stale {
                        let _ = std::fs::remove_dir(&path);
                    } else {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                }
                Err(error) => return Err(format!("pr: acquire review queue lock: {error}")),
            }
        }
        Err("pr: review queue is busy; local handoff remains available for recovery".to_owned())
    }
}

impl Drop for PrQueueLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.path);
    }
}

fn pr_valid_pane_id(value: &str) -> bool {
    value.strip_prefix('%').is_some_and(|digits| !digits.is_empty() && digits.chars().all(|ch| ch.is_ascii_digit()))
}

fn pr_now_epoch() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
        .to_string()
}

fn pr_extract_pr_number(url: &str) -> Option<u64> {
    url.trim_end_matches('/').rsplit('/').next()?.parse().ok()
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
    let branch_issue = pr_extract_issue_num(&branch)
        .ok_or_else(|| "pr: branch must contain issue-<number> for one-issue/one-PR traceability".to_owned())?;
    let delivery = pr_load_delivery(&cwd)?;
    pr_validate_delivery(&delivery, branch_issue)?;
    let title = options.title.clone().unwrap_or_else(|| pr_branch_to_title(&branch));
    pr_validate_text_arg(&title, "title")?;
    let body = pr_render_delivery_body(options.body.as_deref(), &delivery);
    pr_validate_text_arg(&body, "body")?;
    Ok(PrPlan { cwd, branch, base_repo, base_branch: "main".to_owned(), title, body })
}

fn pr_load_delivery(cwd: &std::path::Path) -> Result<DeliveryEvidence, String> {
    let path = cwd.join(".maw/delivery.json");
    let body = std::fs::read_to_string(&path)
        .map_err(|error| format!("pr: required {} is missing or unreadable: {error}", path.display()))?;
    serde_json::from_str(&body)
        .map_err(|error| format!("pr: invalid {}: {error}", path.display()))
}

fn pr_validate_delivery(delivery: &DeliveryEvidence, branch_issue: u64) -> Result<(), String> {
    if delivery.version != 1 {
        return Err(format!("pr: unsupported delivery version {} (expected 1)", delivery.version));
    }
    if delivery.issue != branch_issue {
        return Err(format!(
            "pr: delivery issue {} does not match branch issue {branch_issue}",
            delivery.issue
        ));
    }
    if !matches!(delivery.mode.as_str(), "fast" | "standard" | "swarm" | "discovery") {
        return Err(format!("pr: invalid delivery mode {}", delivery.mode));
    }
    if !matches!(delivery.engine.as_str(), "omx" | "codex") {
        return Err(format!("pr: invalid delivery engine {}", delivery.engine));
    }
    if delivery.verification.commands.is_empty() {
        return Err("pr: delivery verification.commands must contain at least one command".to_owned());
    }
    for command in &delivery.verification.commands {
        if command.command.trim().is_empty() {
            return Err("pr: delivery verification command must be non-empty".to_owned());
        }
        let result = command.result.to_ascii_lowercase();
        if !matches!(result.as_str(), "pass" | "fail" | "blocked") {
            return Err(format!("pr: invalid verification result {}", command.result));
        }
        if result != "pass" {
            return Err(format!(
                "pr: verification command '{}' is {}; PR creation requires pass",
                command.command, command.result
            ));
        }
    }
    if delivery.verification.live_evidence.trim().is_empty() {
        return Err("pr: delivery verification.liveEvidence must be non-empty".to_owned());
    }
    Ok(())
}

fn pr_render_delivery_body(user_body: Option<&str>, delivery: &DeliveryEvidence) -> String {
    let mut sections = Vec::new();
    if let Some(body) = user_body.filter(|body| !body.trim().is_empty()) {
        sections.push(body.trim().to_owned());
    }

    let trace = format!("Closes #{}\nREQ: #{}", delivery.issue, delivery.issue);
    if !sections.iter().any(|section| section.contains(&format!("Closes #{}", delivery.issue)))
        || !sections.iter().any(|section| section.contains(&format!("REQ: #{}", delivery.issue)))
    {
        sections.push(trace);
    }

    let risk_tags = if delivery.risk_tags.is_empty() { "none".to_owned() } else { delivery.risk_tags.join(", ") };
    let spec = delivery.spec.as_deref().unwrap_or("not required");
    let artifacts = if delivery.verification.artifacts.is_empty() {
        "none".to_owned()
    } else {
        delivery.verification.artifacts.join(", ")
    };
    let open_risks = if delivery.verification.open_risks.is_empty() {
        "none".to_owned()
    } else {
        delivery.verification.open_risks.join("; ")
    };
    let commands = delivery
        .verification
        .commands
        .iter()
        .map(|command| format!("- `{}`: {}", command.command, command.result))
        .collect::<Vec<_>>()
        .join("\n");
    sections.push(format!(
        "## Delivery\n\n- Mode: {}\n- Engine: {}\n- Risk tags: {risk_tags}\n- Spec: {spec}\n\n## Verification\n\n{commands}\n- Live evidence: {}\n- Artifacts: {artifacts}\n- Open risks: {open_risks}",
        delivery.mode,
        delivery.engine,
        delivery.verification.live_evidence
    ));
    sections.join("\n\n")
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
        enqueued: Vec<PrReviewRequest>,
        notifications: Vec<String>,
        notification: PrL1Notification,
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
        fn pr_enqueue_review(&mut self, request: &PrReviewRequest) -> Result<(), String> {
            if let Some(existing) = self.enqueued.iter_mut().find(|entry| entry.pr_url == request.pr_url) {
                *existing = request.clone();
            } else {
                self.enqueued.push(request.clone());
            }
            Ok(())
        }
        fn pr_notify_l1(&mut self, _cwd: &std::path::Path, message: &str) -> Result<PrL1Notification, String> {
            self.notifications.push(message.to_owned());
            Ok(self.notification)
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

    fn pr_write_delivery(repo: &std::path::Path, issue: u64) {
        std::fs::create_dir_all(repo.join(".maw")).expect("maw dir");
        let delivery = serde_json::json!({
                "version": 1,
                "issue": issue,
                "mode": "standard",
                "riskTags": ["api"],
                "engine": "omx",
                "spec": format!("specs/{issue}-demo.md"),
                "verification": {
                    "commands": [{"command": "cargo test", "result": "pass"}],
                    "liveEvidence": "VERIFIED-LIVE: focused CLI path",
                    "artifacts": ["target/test.log"],
                    "openRisks": []
                }
            });
        let body = serde_json::to_string_pretty(&delivery).expect("render delivery") + "\n";
        std::fs::write(repo.join(".maw/delivery.json"), body).expect("delivery");
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
        pr_write_delivery(&repo, 140);
        let mut tmux = PrMockTmux { current_path: repo.display().to_string(), ..Default::default() };
        let mut process = PrMockProcess { branch: "agents/issue-140-pr-native".to_owned(), ..Default::default() };

        let output = pr_run(&[], &mut tmux, &mut process).expect("run");

        assert_eq!(output, include_str!("../../tests/fixtures/native-pr/create.stdout"));
        assert_eq!(process.created[0].title, "Issue 140 Pr Native");
        assert!(process.created[0].body.contains("Closes #140\nREQ: #140"));
        assert!(process.created[0].body.contains("## Delivery"));
        assert!(process.created[0].body.contains("- Mode: standard"));
        assert!(process.created[0].body.contains("- Engine: omx"));
        assert!(process.created[0].body.contains("- `cargo test`: pass"));
        assert!(process.created[0].body.contains("VERIFIED-LIVE: focused CLI path"));
        assert_eq!(process.created[0].base_repo, "acme/demo");
        assert_eq!(process.created[0].base_branch, "main");
        assert_eq!(process.enqueued.len(), 1);
        assert!(process.enqueued[0].notified);
        assert_eq!(process.notifications, vec!["[codex] PR #7 ready for issue #140. https://github.com/acme/demo/pull/7"]);
        let request = serde_json::from_str::<PrReviewRequest>(
            &std::fs::read_to_string(repo.join(".maw/l1-review-request.json")).expect("request"),
        )
        .expect("request json");
        assert!(request.notified);
        assert_eq!(request.pr_number, 7);
        assert_eq!(request.status, "notified");
        assert_eq!(request.notifier.as_deref(), Some("maw-pr"));
    }

    #[test]
    fn pr_reads_oracle_metadata_separately_from_legacy_pane() {
        let repo = pr_temp_dir("l1-metadata");
        let metadata = repo.join(".maw");
        std::fs::create_dir_all(&metadata).expect("metadata dir");
        std::fs::write(metadata.join("l1-oracle"), "50-mawjs\n").expect("oracle");
        std::fs::write(metadata.join("l1-pane"), "%42\n").expect("pane");
        assert_eq!(pr_l1_oracle(&repo).as_deref(), Some("50-mawjs"));
        assert_eq!(pr_l1_pane(&repo).as_deref(), Some("%42"));
    }

    #[test]
    fn pr_marks_legacy_pane_fallback() {
        let repo = pr_temp_dir("l1-pane-fallback");
        let _guard = env_test_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _restore = EnvVarRestore::capture("TMUX");
        std::env::set_var("TMUX", "/tmp/tmux,1,0");
        pr_write_delivery(&repo, 32);
        let mut tmux = PrMockTmux { current_path: repo.display().to_string(), ..Default::default() };
        let mut process = PrMockProcess {
            branch: "agents/issue-32-pr-hey-notification".to_owned(),
            notification: PrL1Notification::LegacyPaneFallback,
            ..PrMockProcess::default()
        };

        let output = pr_run(&[], &mut tmux, &mut process).expect("run");

        assert!(output.contains("L1 oracle metadata unavailable; used legacy .maw/l1-pane fallback"), "{output}");
        assert!(output.contains("\x1b[32m✅\x1b[0m L1 notified"), "{output}");
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
        pr_write_delivery(&repo, 42);
        let options = PrOptions { window: None, title: Some("Custom".to_owned()), body: Some("Body".to_owned()), show_current: false };
        let plan = pr_build_plan(repo, "agents/issue-42-demo".to_owned(), "deachawatss/maw-rs".to_owned(), &options).expect("plan");
        assert_eq!(plan.title, "Custom");
        assert!(plan.body.starts_with("Body\n\nCloses #42\nREQ: #42"));
        let error = pr_build_plan(std::path::PathBuf::from("/tmp"), String::new(), "deachawatss/maw-rs".to_owned(), &options).expect_err("detached");
        assert!(error.contains("detached HEAD"));
    }

    #[test]
    fn pr_requires_valid_delivery_evidence_matching_branch_issue() {
        let repo = pr_temp_dir("delivery-required");
        let options = PrOptions { window: None, title: None, body: None, show_current: false };

        let missing = pr_build_plan(
            repo.clone(),
            "agents/issue-42-demo".to_owned(),
            "deachawatss/maw-rs".to_owned(),
            &options,
        )
        .expect_err("missing delivery blocked");
        assert!(missing.contains(".maw/delivery.json"), "{missing}");

        pr_write_delivery(&repo, 41);
        let mismatch = pr_build_plan(
            repo.clone(),
            "agents/issue-42-demo".to_owned(),
            "deachawatss/maw-rs".to_owned(),
            &options,
        )
        .expect_err("mismatched issue blocked");
        assert!(mismatch.contains("delivery issue 41 does not match branch issue 42"), "{mismatch}");

        pr_write_delivery(&repo, 42);
        let path = repo.join(".maw/delivery.json");
        let mut delivery: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("delivery body")).expect("delivery json");
        delivery["engine"] = "claude".into();
        std::fs::write(&path, serde_json::to_string_pretty(&delivery).expect("render delivery")).expect("delivery");
        let invalid_engine = pr_build_plan(
            repo,
            "agents/issue-42-demo".to_owned(),
            "deachawatss/maw-rs".to_owned(),
            &options,
        )
        .expect_err("invalid engine blocked");
        assert!(invalid_engine.contains("invalid delivery engine claude"), "{invalid_engine}");
    }

    #[test]
    fn pr_global_review_queue_upserts_by_repo_and_url() {
        let _guard = env_test_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _restore = EnvVarRestore::capture("MAW_STATE_DIR");
        let state = pr_temp_dir("review-queue");
        std::env::set_var("MAW_STATE_DIR", &state);
        let mut request = PrReviewRequest {
            version: 1,
            pr_url: "https://github.com/acme/demo/pull/7".to_owned(),
            pr_number: 7,
            repo: "acme/demo".to_owned(),
            branch: "agents/issue-7-demo".to_owned(),
            status: "pending".to_owned(),
            notified: false,
            notified_at: None,
            notifier: None,
        };

        pr_enqueue_global_review(&request).expect("enqueue pending");
        request.notified = true;
        request.status = "notified".to_owned();
        request.notified_at = Some("123".to_owned());
        request.notifier = Some("maw-pr".to_owned());
        pr_enqueue_global_review(&request).expect("upsert notified");

        let rows = std::fs::read_to_string(state.join("pr-queue.jsonl")).expect("queue");
        assert_eq!(rows.lines().count(), 1);
        assert_eq!(serde_json::from_str::<PrReviewRequest>(rows.trim()).expect("row"), request);
    }

    #[test]
    fn pr_targets_origin_fork_main_and_rejects_soul_brews_upstream() {
        let _guard = env_test_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _restore = EnvVarRestore::capture("TMUX");
        std::env::set_var("TMUX", "/tmp/tmux,1,0");
        let repo = pr_temp_dir("fork-target");
        pr_write_delivery(&repo, 17);
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
