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
    reconcile: bool,
    quiet: bool,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeliveryCommandResult {
    Pass,
    Skip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrDeliveryCommandOutcome {
    Passed,
    Exited(Option<i32>),
    TimedOut { timeout_ms: u64 },
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
    #[serde(default)]
    l1_oracle: Option<String>,
    #[serde(default)]
    l1_pane: Option<String>,
    #[serde(default)]
    reconcile_attempts: u8,
    #[serde(default)]
    last_reconcile_error: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum PrGithubState {
    #[default]
    Open,
    Merged,
    Closed,
}

const PR_RECONCILE_MAX_ATTEMPTS: u8 = 3;
const PR_GH_CREATE_MAX_ATTEMPTS: u8 = 3;
const PR_GH_CREATE_INITIAL_BACKOFF_MS: u64 = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
enum PrReconciliationUpdate {
    Archive(String),
    Keep,
    Failed(String),
}

trait PrTmux {
    fn pr_current_path(&mut self) -> Result<String, String>;
    fn pr_current_session(&mut self) -> Result<String, String>;
    fn pr_window_path(&mut self, target: &str) -> Result<String, String>;
}

struct PrNativeTmux;

impl PrTmux for PrNativeTmux {
    fn pr_current_path(&mut self) -> Result<String, String> {
        // The caller's own working directory is ground truth. Asking tmux via
        // `display-message` with no target resolves to the attached client's
        // active pane whenever TMUX_PANE is absent (codex background
        // terminals scrub it), silently reading another pane's repo/branch.
        std::env::current_dir()
            .map(|path| path.display().to_string())
            .map_err(|error| format!("could not detect working directory: {error}"))
    }

    fn pr_current_session(&mut self) -> Result<String, String> {
        // Pin the lookup to the invoking pane when TMUX_PANE is available;
        // untargeted display-message falls back to the focused client's pane.
        match std::env::var("TMUX_PANE") {
            Ok(pane) if !pane.is_empty() => {
                pr_tmux_output(&["display-message", "-t", &pane, "-p", "#{session_name}"])
            }
            _ => pr_tmux_output(&["display-message", "-p", "#{session_name}"]),
        }
    }

    fn pr_window_path(&mut self, target: &str) -> Result<String, String> {
        pr_validate_tmux_target(target, "window target")?;
        pr_tmux_output(&["display-message", "-t", target, "-p", "#{pane_current_path}"])
    }
}

trait PrProcess {
    fn pr_git_branch(&mut self, cwd: &std::path::Path) -> Result<String, String>;
    fn pr_git_remote_url(&mut self, cwd: &std::path::Path, remote: &str) -> Result<String, String>;
    fn pr_rerun_delivery_command(
        &mut self,
        cwd: &std::path::Path,
        command: &str,
    ) -> Result<PrDeliveryCommandOutcome, String>;
    fn pr_gh_create(&mut self, plan: &PrPlan) -> Result<String, String>;
    fn pr_gh_view_current(&mut self, cwd: &std::path::Path) -> Result<String, String>;
    fn pr_gh_review_state(&mut self, repo: &str, pr_number: u64) -> Result<PrGithubState, String>;
    fn pr_reconcile_review_queue(&mut self, quiet: bool) -> Result<String, String>;
    fn pr_enqueue_review(&mut self, request: &PrReviewRequest) -> Result<(), String>;
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
            .map_err(|error| format!("git branch --show-current: {error}"))?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
        } else {
            Err(pr_command_failure("git branch --show-current", &output))
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
            .map_err(|error| format!("git remote get-url {remote}: {error}"))?;
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned());
        }
        Err(pr_command_failure(&format!("git remote get-url {remote}"), &output))
    }

    fn pr_rerun_delivery_command(
        &mut self,
        cwd: &std::path::Path,
        command: &str,
    ) -> Result<PrDeliveryCommandOutcome, String> {
        pr_rerun_local_delivery_command(cwd, command)
    }

    fn pr_gh_create(&mut self, plan: &PrPlan) -> Result<String, String> {
        pr_validate_cwd(&plan.cwd)?;
        let mut attempts_remaining = PR_GH_CREATE_MAX_ATTEMPTS;
        let mut retry_index = 0;
        loop {
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
                .map_err(|error| format!("gh pr create: {error}"))?;
            if output.status.success() {
                return Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned());
            }
            attempts_remaining = attempts_remaining.saturating_sub(1);
            let error = pr_command_failure("gh pr create", &output);
            if attempts_remaining == 0 || !pr_is_transient_gh_create_failure(&output.stderr) {
                return Err(error);
            }
            std::thread::sleep(pr_gh_create_retry_delay(retry_index));
            retry_index = retry_index.saturating_add(1);
        }
    }

    fn pr_gh_view_current(&mut self, cwd: &std::path::Path) -> Result<String, String> {
        pr_validate_cwd(cwd)?;
        let output = std::process::Command::new("gh")
            .current_dir(cwd)
            .args(["pr", "view", "--json", "number,title,url", "--jq", "#\\(.number) \\(.title) \\(.url)"])
            .output()
            .map_err(|error| format!("gh pr view: {error}"))?;
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned());
        }
        Err(pr_command_failure("gh pr view", &output))
    }

    fn pr_gh_review_state(&mut self, repo: &str, pr_number: u64) -> Result<PrGithubState, String> {
        pr_validate_review_repo(repo)?;
        if pr_number == 0 {
            return Err("pr: queued PR number must be positive".to_owned());
        }
        let output = std::process::Command::new("gh")
            .args([
                "pr",
                "view",
                &pr_number.to_string(),
                "--repo",
                repo,
                "--json",
                "state",
                "--jq",
                ".state",
            ])
            .output()
            .map_err(|error| format!("pr: view queued PR #{pr_number}: {error}"))?;
        if !output.status.success() {
            return Err(pr_command_failure(&format!("pr: view queued PR #{pr_number}"), &output));
        }
        match String::from_utf8_lossy(&output.stdout).trim() {
            "OPEN" => Ok(PrGithubState::Open),
            "MERGED" => Ok(PrGithubState::Merged),
            "CLOSED" => Ok(PrGithubState::Closed),
            state => Err(format!("pr: queued PR #{pr_number} returned unsupported state {state:?}")),
        }
    }

    fn pr_reconcile_review_queue(&mut self, quiet: bool) -> Result<String, String> {
        pr_reconcile_reviews(self, quiet)
    }

    fn pr_enqueue_review(&mut self, request: &PrReviewRequest) -> Result<(), String> {
        pr_enqueue_global_review(request)
    }

}

fn pr_command_failure(command: &str, output: &std::process::Output) -> String {
    let exit_code = output.status.code().unwrap_or(1);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if stderr.trim().is_empty() {
        format!("{command} failed (exit {exit_code})")
    } else {
        format!("{command} failed (exit {exit_code}): {stderr}")
    }
}

fn pr_is_transient_gh_create_failure(stderr: &[u8]) -> bool {
    let message = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    let has_server_error = message
        .split(|character: char| !character.is_ascii_digit())
        .filter_map(|part| part.parse::<u16>().ok())
        .any(|status| (500..=599).contains(&status));

    message.contains("tls handshake")
        || message.contains("could not resolve host")
        || message.contains("temporary failure in name resolution")
        || message.contains("connection reset")
        || message.contains("connection refused")
        || message.contains("network is unreachable")
        || message.contains("context deadline exceeded")
        || message.contains("i/o timeout")
        || (message.contains("graphql") && (message.contains("timeout") || message.contains("5xx")))
        || has_server_error
}

fn pr_gh_create_retry_delay(retry_index: u8) -> std::time::Duration {
    std::time::Duration::from_millis(
        PR_GH_CREATE_INITIAL_BACKOFF_MS.saturating_mul(1_u64 << u32::from(retry_index)),
    )
}

fn run_pr_command(argv: &[String]) -> CliOutput {
    match pr_run(argv, &mut PrNativeTmux, &mut PrNativeProcess) {
        Ok(stdout) => CliOutput { code: 0, stdout, stderr: String::new() },
        Err(message) => CliOutput { code: 1, stdout: String::new(), stderr: format!("{message}\n") },
    }
}

fn pr_run<T: PrTmux, P: PrProcess>(argv: &[String], tmux: &mut T, process: &mut P) -> Result<String, String> {
    let options = pr_parse_args(argv)?;
    if options.reconcile {
        return pr_reconcile_reviews(process, options.quiet);
    }
    let cwd = pr_resolve_cwd(options.window.as_deref(), tmux)?;
    if options.show_current {
        return process.pr_gh_view_current(&cwd).map(|line| format!("{line}\n"));
    }
    process.pr_reconcile_review_queue(true)?;
    let branch = process.pr_git_branch(&cwd)?;
    let origin_url = process.pr_git_remote_url(&cwd, "origin")?;
    let base_repo = pr_github_repo_from_remote(&origin_url)?;
    let plan = pr_build_plan(cwd, branch, base_repo, &options, process)?;
    let mut out = pr_render_start(&plan);
    let url = process.pr_gh_create(&plan)?;
    let _ = writeln!(out, "\x1b[32m✅\x1b[0m {url}");
    let pr_number = pr_extract_pr_number(&url)
        .ok_or_else(|| format!("pr: could not determine PR number from gh response: {url}"))?;
    let request = PrReviewRequest {
        version: 1,
        pr_url: url.clone(),
        pr_number,
        repo: plan.base_repo.clone(),
        branch: plan.branch.clone(),
        status: "pending".to_owned(),
        notified: false,
        notified_at: None,
        notifier: None,
        l1_oracle: pr_l1_oracle(&plan.cwd),
        l1_pane: pr_l1_pane(&plan.cwd),
        reconcile_attempts: 0,
        last_reconcile_error: None,
    };
    pr_write_review_request(&plan.cwd, &request)?;
    process.pr_enqueue_review(&request)?;
    if let Err(error) = l2_emit_pr_event(&plan.cwd, pr_number, &url) {
        let _ = writeln!(out, "\x1b[33m⚠\x1b[0m {error}; PR remains pending in the review queue");
    } else {
        let _ = writeln!(out, "\x1b[33m⚠\x1b[0m L1 handoff queued for the next hook drain");
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
    let root = pr_review_queue_root()?;
    let _lock = PrQueueLock::acquire(&root)?;
    let mut seen = std::collections::HashSet::new();
    let key = pr_review_queue_key(request);
    let mut replaced = false;
    let mut lines = Vec::new();
    for line in pr_read_queue_lines(&root, "pr-queue.jsonl")? {
        match serde_json::from_str::<PrReviewRequest>(&line) {
            Ok(entry) if pr_review_queue_key(&entry) == key => {
                if !replaced {
                    lines.push(pr_render_queue_row(request)?);
                    replaced = true;
                }
            }
            Ok(entry) if seen.insert(pr_review_queue_key(&entry)) => lines.push(line),
            Ok(_) => {}
            Err(_) => lines.push(line),
        }
    }
    if !replaced {
        lines.push(pr_render_queue_row(request)?);
    }
    pr_write_queue_lines(&root, "pr-queue.jsonl", &lines)
}

fn pr_reconcile_reviews<P: PrProcess>(process: &mut P, quiet: bool) -> Result<String, String> {
    let root = pr_review_queue_root()?;
    let queued = pr_load_global_reviews(&root)?;
    let mut updates = std::collections::BTreeMap::new();
    let mut out = String::new();

    for request in queued {
        match process.pr_gh_review_state(&request.repo, request.pr_number) {
            Ok(PrGithubState::Merged) => {
                updates.insert(pr_review_queue_key(&request), PrReconciliationUpdate::Archive("merged".to_owned()));
                if !quiet {
                    let _ = writeln!(out, "\x1b[32m✅\x1b[0m PR #{} merged; archiving queued handoff", request.pr_number);
                }
            }
            Ok(PrGithubState::Closed) => {
                updates.insert(pr_review_queue_key(&request), PrReconciliationUpdate::Archive("closed".to_owned()));
                if !quiet {
                    let _ = writeln!(out, "\x1b[32m✅\x1b[0m PR #{} closed; archiving queued handoff", request.pr_number);
                }
            }
            Ok(PrGithubState::Open) => {
                updates.insert(pr_review_queue_key(&request), PrReconciliationUpdate::Keep);
                if !quiet {
                    let _ = writeln!(out, "PR #{} is still open; durable handoff retained", request.pr_number);
                }
            }
            Err(error) => {
                updates.insert(pr_review_queue_key(&request), PrReconciliationUpdate::Failed(error.clone()));
                if !quiet {
                    let _ = writeln!(
                        out,
                        "\x1b[33m⚠\x1b[0m PR #{} reconciliation deferred: {error}",
                        request.pr_number
                    );
                }
            }
        }
    }

    let archived_count = pr_finalize_global_reconciliation(&root, &updates)?;
    if !quiet && out.is_empty() {
        let _ = writeln!(out, "No PR handoffs to reconcile.");
    } else if !quiet && archived_count > 0 {
        let _ = writeln!(out, "Archived {archived_count} completed PR handoff(s).");
    }
    Ok(out)
}

fn pr_review_queue_root() -> Result<std::path::PathBuf, String> {
    let root = std::env::var_os("MAW_STATE_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".maw")))
        .ok_or_else(|| "pr: HOME/MAW_STATE_DIR unavailable for review queue".to_owned())?;
    std::fs::create_dir_all(&root).map_err(|error| format!("pr: create review queue dir: {error}"))?;
    Ok(root)
}

fn pr_load_global_reviews(root: &std::path::Path) -> Result<Vec<PrReviewRequest>, String> {
    let _lock = PrQueueLock::acquire(root)?;
    let mut seen = std::collections::HashSet::new();
    let mut queued = Vec::new();
    for line in pr_read_queue_lines(root, "pr-queue.jsonl")? {
        let Ok(request) = serde_json::from_str::<PrReviewRequest>(&line) else { continue };
        if request.status != "unresolvable" && seen.insert(pr_review_queue_key(&request)) {
            queued.push(request);
        }
    }
    Ok(queued)
}

fn pr_finalize_global_reconciliation(
    root: &std::path::Path,
    updates: &std::collections::BTreeMap<String, PrReconciliationUpdate>,
) -> Result<usize, String> {
    let _lock = PrQueueLock::acquire(root)?;
    let mut retained = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut completed = Vec::new();

    for line in pr_read_queue_lines(root, "pr-queue.jsonl")? {
        match serde_json::from_str::<PrReviewRequest>(&line) {
            Ok(mut request) => {
                let key = pr_review_queue_key(&request);
                match updates.get(&key) {
                    Some(PrReconciliationUpdate::Archive(status)) => {
                        request.status.clone_from(status);
                        request.notified = false;
                        request.notified_at = None;
                        request.notifier = None;
                        if seen.insert(key) {
                            completed.push(request);
                        }
                    }
                    Some(PrReconciliationUpdate::Keep) => {
                        request.reconcile_attempts = 0;
                        request.last_reconcile_error = None;
                        if seen.insert(key) {
                            retained.push(pr_render_queue_row(&request)?);
                        }
                    }
                    Some(PrReconciliationUpdate::Failed(error)) => {
                        request.reconcile_attempts = request.reconcile_attempts.saturating_add(1);
                        request.last_reconcile_error = Some(error.clone());
                        if request.reconcile_attempts >= PR_RECONCILE_MAX_ATTEMPTS {
                            request.status.clear();
                            request.status.push_str("unresolvable");
                        }
                        if seen.insert(key) {
                            retained.push(pr_render_queue_row(&request)?);
                        }
                    }
                    None if seen.insert(key) => retained.push(line),
                    None => {}
                }
            }
            Err(_) => retained.push(line),
        }
    }

    if !completed.is_empty() {
        let mut archived_keys = std::collections::HashSet::new();
        let mut archived_lines = Vec::new();
        for line in pr_read_queue_lines(root, "pr-queue.jsonl.archived")? {
            match serde_json::from_str::<PrReviewRequest>(&line) {
                Ok(request) if archived_keys.insert(pr_review_queue_key(&request)) => archived_lines.push(line),
                Ok(_) => {}
                Err(_) => archived_lines.push(line),
            }
        }
        for request in completed.iter().filter(|request| archived_keys.insert(pr_review_queue_key(request))) {
            archived_lines.push(pr_render_queue_row(request)?);
        }
        pr_write_queue_lines(root, "pr-queue.jsonl.archived", &archived_lines)?;
    }
    pr_write_queue_lines(root, "pr-queue.jsonl", &retained)?;
    Ok(completed.len())
}

fn pr_review_queue_key(request: &PrReviewRequest) -> String {
    format!("{}#{}", request.repo.trim().to_ascii_lowercase(), request.pr_number)
}

fn pr_read_queue_lines(root: &std::path::Path, name: &str) -> Result<Vec<String>, String> {
    let path = root.join(name);
    match std::fs::read_to_string(&path) {
        Ok(body) => Ok(body.lines().map(str::to_owned).collect()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(format!("pr: read {}: {error}", path.display())),
    }
}

fn pr_render_queue_row(request: &PrReviewRequest) -> Result<String, String> {
    serde_json::to_string(request).map_err(|error| format!("pr: render queue row: {error}"))
}

fn pr_write_queue_lines(root: &std::path::Path, name: &str, lines: &[String]) -> Result<(), String> {
    let path = root.join(name);
    let body = if lines.is_empty() { String::new() } else { lines.join("\n") + "\n" };
    let tmp = root.join(format!(".{name}.{}.tmp", std::process::id()));
    std::fs::write(&tmp, body).map_err(|error| format!("pr: write {}: {error}", path.display()))?;
    std::fs::rename(&tmp, &path).map_err(|error| format!("pr: replace {}: {error}", path.display()))
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

fn pr_extract_pr_number(url: &str) -> Option<u64> {
    url.trim_end_matches('/').rsplit('/').next()?.parse().ok()
}

fn pr_parse_args(argv: &[String]) -> Result<PrOptions, String> {
    let mut options = PrOptions { window: None, title: None, body: None, show_current: false, reconcile: false, quiet: false };
    let mut index = 0_usize;
    while let Some(arg) = argv.get(index) {
        match arg.as_str() {
            "--help" | "-h" => return Err(pr_usage().to_owned()),
            "--show-current" => { options.show_current = true; index += 1; }
            "--reconcile" => { options.reconcile = true; index += 1; }
            "--quiet" => { options.quiet = true; index += 1; }
            "--title" => { options.title = Some(pr_required_value(argv, index, "--title")?); index += 2; }
            value if value.starts_with("--title=") => { options.title = Some(value["--title=".len()..].to_owned()); index += 1; }
            "--body" => { options.body = Some(pr_required_value(argv, index, "--body")?); index += 2; }
            value if value.starts_with("--body=") => { options.body = Some(value["--body=".len()..].to_owned()); index += 1; }
            value if value.starts_with('-') => return Err(format!("pr: unknown argument {value}")),
            "reconcile" => { options.reconcile = true; index += 1; }
            value => { pr_set_window(&mut options, value)?; index += 1; }
        }
    }
    if (options.reconcile && (options.window.is_some() || options.title.is_some() || options.body.is_some() || options.show_current))
        || (options.quiet && !options.reconcile)
    {
        return Err(pr_usage().to_owned());
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
    process: &mut impl PrProcess,
) -> Result<PrPlan, String> {
    pr_validate_branch(&branch)?;
    pr_validate_base_repo(&base_repo)?;
    let branch_issue = pr_extract_issue_num(&branch)
        .ok_or_else(|| "pr: branch must contain issue-<number> for one-issue/one-PR traceability".to_owned())?;
    let delivery = pr_load_delivery(&cwd)?;
    pr_validate_delivery(&delivery, branch_issue)?;
    pr_rerun_delivery_commands(&delivery, &cwd, process)?;
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
    // The WF pre-guard.sh hook remains the engine-neutral shape-only fallback for
    // direct `gh pr create` and older binaries. Native `maw pr` owns re-running
    // successful local verification commands after this validation succeeds.
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
        let _ = pr_delivery_command_result(&command.result)?;
    }
    if delivery.verification.live_evidence.trim().is_empty() {
        return Err("pr: delivery verification.liveEvidence must be non-empty".to_owned());
    }
    Ok(())
}

fn pr_delivery_command_result(result: &str) -> Result<DeliveryCommandResult, String> {
    let result = result.trim();
    let normalized = result.to_ascii_lowercase();
    if normalized == "pass" || normalized.starts_with("pass:") {
        return Ok(DeliveryCommandResult::Pass);
    }
    if normalized == "skip" || normalized.starts_with("skip:") {
        return Ok(DeliveryCommandResult::Skip);
    }
    if matches!(normalized.as_str(), "fail" | "blocked") {
        return Err(format!("pr: verification result {result} blocks PR creation"));
    }
    Err(format!("pr: invalid verification result {result}"))
}

fn pr_rerun_delivery_commands(
    delivery: &DeliveryEvidence,
    cwd: &std::path::Path,
    process: &mut impl PrProcess,
) -> Result<(), String> {
    for delivery_command in &delivery.verification.commands {
        if pr_delivery_command_result(&delivery_command.result)? == DeliveryCommandResult::Skip {
            continue;
        }
        match process.pr_rerun_delivery_command(cwd, &delivery_command.command) {
            Ok(PrDeliveryCommandOutcome::Passed) => {}
            Ok(PrDeliveryCommandOutcome::Exited(Some(exit_code))) => {
                return Err(format!(
                    "pr: verification command '{}' failed on re-run (observed exit code {exit_code})",
                    delivery_command.command
                ));
            }
            Ok(PrDeliveryCommandOutcome::Exited(None)) => {
                return Err(format!(
                    "pr: verification command '{}' terminated on re-run without an exit code",
                    delivery_command.command
                ));
            }
            Ok(PrDeliveryCommandOutcome::TimedOut { timeout_ms }) => {
                return Err(format!(
                    "pr: verification command '{}' timed out after {timeout_ms}ms; re-run verification blocks PR",
                    delivery_command.command
                ));
            }
            Err(error) => {
                return Err(format!(
                    "pr: verification command '{}' could not be re-run: {error}; use skip: <reason> or defer it to openRisks",
                    delivery_command.command
                ));
            }
        }
    }
    Ok(())
}

const PR_DELIVERY_COMMAND_TIMEOUT_DEFAULT_MS: u64 = 300_000;
const PR_DELIVERY_COMMAND_TIMEOUT_ENV: &str = "MAW_PR_VERIFICATION_TIMEOUT_MS";

fn pr_delivery_command_timeout_ms() -> u64 {
    std::env::var(PR_DELIVERY_COMMAND_TIMEOUT_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|millis| (100..=900_000).contains(millis))
        .unwrap_or(PR_DELIVERY_COMMAND_TIMEOUT_DEFAULT_MS)
}

fn pr_rerun_local_delivery_command(
    cwd: &std::path::Path,
    command: &str,
) -> Result<PrDeliveryCommandOutcome, String> {
    pr_validate_cwd(cwd)?;
    let timeout_ms = pr_delivery_command_timeout_ms();
    let timeout = std::time::Duration::from_millis(timeout_ms);
    let mut child = std::process::Command::new("sh")
        .args(["-c", command])
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| format!("spawn failed: {error}"))?;
    let started_at = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(PrDeliveryCommandOutcome::Passed),
            Ok(Some(status)) => return Ok(PrDeliveryCommandOutcome::Exited(status.code())),
            Ok(None) if started_at.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Ok(PrDeliveryCommandOutcome::TimedOut { timeout_ms });
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(25)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("wait failed: {error}"));
            }
        }
    }
}

fn pr_render_delivery_body(user_body: Option<&str>, delivery: &DeliveryEvidence) -> String {
    let mut sections = Vec::new();
    if let Some(body) = user_body.filter(|body| !body.trim().is_empty()) {
        sections.push(body.trim().to_owned());
    }

    let trace = wind_pr_default_trace(delivery.issue);
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
        .map(|command| {
            let suffix = match pr_delivery_command_result(&command.result) {
                Ok(DeliveryCommandResult::Skip) => " (not re-run)",
                Ok(DeliveryCommandResult::Pass) | Err(_) => "",
            };
            format!("- `{}`: {}{suffix}", command.command, command.result)
        })
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
    "usage: maw pr [window] [--title <title>] [--body <body>] [--show-current]\n       maw pr reconcile [--quiet] | --reconcile [--quiet]"
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
    pr_validate_review_repo(value)?;
    let (owner, _) = value.split_once('/').expect("validated owner/repo");
    if owner.eq_ignore_ascii_case("Soul-Brews-Studio") {
        return Err(format!(
            "pr: refusing to create PR against read-only upstream {value}; set origin to a fork"
        ));
    }
    Ok(())
}

fn pr_validate_review_repo(value: &str) -> Result<(), String> {
    let (owner, repo) = value.split_once('/').ok_or_else(|| "pr: base repo must use owner/repo".to_owned())?;
    pr_validate_github_segment(owner, "owner")?;
    pr_validate_github_segment(repo, "repo")?;
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

    #[test]
    fn pr_native_current_path_is_the_callers_cwd_not_a_tmux_pane() {
        // Regression for issue #92: resolving the caller's own cwd through
        // `tmux display-message` (no target) returns the attached client's
        // active pane when TMUX_PANE is absent — another pane's repo/branch.
        let cwd = std::env::current_dir().expect("cwd").display().to_string();
        let mut tmux = PrNativeTmux;
        assert_eq!(tmux.pr_current_path().expect("current path"), cwd);
    }

    #[derive(Default)]
    struct PrMockProcess {
        branch: String,
        origin_url: String,
        rerun_commands: Vec<String>,
        rerun_outcomes: std::collections::VecDeque<PrDeliveryCommandOutcome>,
        created: Vec<PrPlan>,
        viewed: Vec<String>,
        review_state_results: std::collections::VecDeque<Result<PrGithubState, String>>,
        reconcile_quiet: Vec<bool>,
        enqueued: Vec<PrReviewRequest>,
    }

    impl PrProcess for PrMockProcess {
        fn pr_git_branch(&mut self, cwd: &std::path::Path) -> Result<String, String> {
            Ok(if self.branch.is_empty() { cwd.file_name().unwrap().to_string_lossy().into_owned() } else { self.branch.clone() })
        }
        fn pr_git_remote_url(&mut self, _cwd: &std::path::Path, remote: &str) -> Result<String, String> {
            assert_eq!(remote, "origin");
            Ok(if self.origin_url.is_empty() { "https://github.com/acme/demo.git".to_owned() } else { self.origin_url.clone() })
        }
        fn pr_rerun_delivery_command(
            &mut self,
            _cwd: &std::path::Path,
            command: &str,
        ) -> Result<PrDeliveryCommandOutcome, String> {
            self.rerun_commands.push(command.to_owned());
            Ok(self.rerun_outcomes.pop_front().unwrap_or(PrDeliveryCommandOutcome::Passed))
        }
        fn pr_gh_create(&mut self, plan: &PrPlan) -> Result<String, String> {
            self.created.push(plan.clone());
            Ok("https://github.com/acme/demo/pull/7".to_owned())
        }
        fn pr_gh_view_current(&mut self, cwd: &std::path::Path) -> Result<String, String> {
            self.viewed.push(cwd.display().to_string());
            Ok("#7 Demo https://github.com/acme/demo/pull/7".to_owned())
        }
        fn pr_gh_review_state(&mut self, _repo: &str, _pr_number: u64) -> Result<PrGithubState, String> {
            self.review_state_results.pop_front().unwrap_or(Ok(PrGithubState::Open))
        }
        fn pr_reconcile_review_queue(&mut self, quiet: bool) -> Result<String, String> {
            self.reconcile_quiet.push(quiet);
            Ok(String::new())
        }
        fn pr_enqueue_review(&mut self, request: &PrReviewRequest) -> Result<(), String> {
            if let Some(existing) = self.enqueued.iter_mut().find(|entry| entry.pr_url == request.pr_url) {
                *existing = request.clone();
            } else {
                self.enqueued.push(request.clone());
            }
            Ok(())
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
    fn pr_recognizes_only_transient_gh_create_failures() {
        for stderr in [
            "TLS handshake timeout",
            "Post https://api.github.com/graphql: timeout",
            "GraphQL 5xx response",
            "HTTP 503 Service Unavailable",
            "could not resolve host: api.github.com",
        ] {
            assert!(pr_is_transient_gh_create_failure(stderr.as_bytes()), "{stderr}");
        }

        for stderr in ["authentication required", "validation failed: head branch is missing"] {
            assert!(!pr_is_transient_gh_create_failure(stderr.as_bytes()), "{stderr}");
        }
    }

    #[cfg(unix)]
    fn pr_write_executable(path: &std::path::Path, body: &str) {
        use std::os::unix::fs::PermissionsExt;

        std::fs::write(path, body).expect("write executable");
        let mut permissions = std::fs::metadata(path).expect("executable metadata").permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).expect("make executable");
    }

    #[cfg(unix)]
    fn pr_native_test_plan(cwd: &std::path::Path) -> PrPlan {
        PrPlan {
            cwd: cwd.to_path_buf(),
            branch: "agents/issue-109-pr-stderr-retry".to_owned(),
            base_repo: "deachawatss/maw-rs".to_owned(),
            base_branch: "main".to_owned(),
            title: "Issue 109 Pr Retry".to_owned(),
            body: "Closes #109".to_owned(),
        }
    }

    #[cfg(unix)]
    #[test]
    fn pr_native_gh_create_surfaces_stderr_on_failure() {
        let _guard = env_test_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _path = EnvVarRestore::capture("PATH");
        let root = pr_temp_dir("native-gh-stderr");
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).expect("bin dir");
        pr_write_executable(
            &bin.join("gh"),
            "#!/bin/sh\nprintf '%s\\n' 'Post https://api.github.com/graphql: authentication required' >&2\nexit 1\n",
        );
        std::env::set_var("PATH", &bin);
        let mut process = PrNativeProcess;

        let error = process.pr_gh_create(&pr_native_test_plan(&root)).expect_err("gh create fails");

        assert!(error.contains("exit 1"), "{error}");
        assert!(error.contains("authentication required"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn pr_native_gh_create_retries_transient_failure_until_success() {
        let _guard = env_test_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _path = EnvVarRestore::capture("PATH");
        let _attempts = EnvVarRestore::capture("MAW_PR_TEST_GH_ATTEMPTS");
        let root = pr_temp_dir("native-gh-retry");
        let bin = root.join("bin");
        let attempts = root.join("attempts");
        std::fs::create_dir_all(&bin).expect("bin dir");
        pr_write_executable(
            &bin.join("gh"),
            "#!/bin/sh\ncount=0\nif [ -f \"$MAW_PR_TEST_GH_ATTEMPTS\" ]; then read -r count < \"$MAW_PR_TEST_GH_ATTEMPTS\"; fi\ncount=$((count + 1))\nprintf '%s\\n' \"$count\" > \"$MAW_PR_TEST_GH_ATTEMPTS\"\nif [ \"$count\" -lt 3 ]; then printf '%s\\n' 'TLS handshake timeout' >&2; exit 1; fi\nprintf '%s\\n' 'https://github.com/deachawatss/maw-rs/pull/109'\n",
        );
        std::env::set_var("PATH", &bin);
        std::env::set_var("MAW_PR_TEST_GH_ATTEMPTS", &attempts);
        let mut process = PrNativeProcess;

        let url = process.pr_gh_create(&pr_native_test_plan(&root)).expect("transient retry succeeds");

        assert_eq!(url, "https://github.com/deachawatss/maw-rs/pull/109");
        assert_eq!(std::fs::read_to_string(attempts).expect("attempt count").trim(), "3");
    }

    #[cfg(unix)]
    #[test]
    fn pr_native_gh_create_fails_fast_for_non_transient_error() {
        let _guard = env_test_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _path = EnvVarRestore::capture("PATH");
        let _attempts = EnvVarRestore::capture("MAW_PR_TEST_GH_ATTEMPTS");
        let root = pr_temp_dir("native-gh-no-retry");
        let bin = root.join("bin");
        let attempts = root.join("attempts");
        std::fs::create_dir_all(&bin).expect("bin dir");
        pr_write_executable(
            &bin.join("gh"),
            "#!/bin/sh\ncount=0\nif [ -f \"$MAW_PR_TEST_GH_ATTEMPTS\" ]; then read -r count < \"$MAW_PR_TEST_GH_ATTEMPTS\"; fi\ncount=$((count + 1))\nprintf '%s\\n' \"$count\" > \"$MAW_PR_TEST_GH_ATTEMPTS\"\nprintf '%s\\n' 'authentication required' >&2\nexit 1\n",
        );
        std::env::set_var("PATH", &bin);
        std::env::set_var("MAW_PR_TEST_GH_ATTEMPTS", &attempts);
        let mut process = PrNativeProcess;

        let error = process.pr_gh_create(&pr_native_test_plan(&root)).expect_err("auth must fail fast");

        assert!(error.contains("authentication required"), "{error}");
        assert_eq!(std::fs::read_to_string(attempts).expect("attempt count").trim(), "1");
    }

    #[cfg(unix)]
    #[test]
    fn pr_native_process_surfaces_stderr_for_other_git_and_gh_failures() {
        let _guard = env_test_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _path = EnvVarRestore::capture("PATH");
        let root = pr_temp_dir("native-command-stderr");
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).expect("bin dir");
        pr_write_executable(&bin.join("git"), "#!/bin/sh\nprintf '%s\\n' 'git transport unavailable' >&2\nexit 128\n");
        pr_write_executable(&bin.join("gh"), "#!/bin/sh\nprintf '%s\\n' 'GraphQL service unavailable' >&2\nexit 1\n");
        std::env::set_var("PATH", &bin);
        let mut process = PrNativeProcess;

        let branch = process.pr_git_branch(&root).expect_err("git branch fails");
        let remote = process.pr_git_remote_url(&root, "origin").expect_err("git remote fails");
        let view = process.pr_gh_view_current(&root).expect_err("gh view fails");
        let review = process.pr_gh_review_state("deachawatss/maw-rs", 109).expect_err("gh review fails");

        assert!(branch.contains("git transport unavailable"), "{branch}");
        assert!(remote.contains("git transport unavailable"), "{remote}");
        assert!(view.contains("GraphQL service unavailable"), "{view}");
        assert!(review.contains("GraphQL service unavailable"), "{review}");
    }

    fn pr_write_delivery(repo: &std::path::Path, issue: u64) -> EnvVarRestore {
        std::fs::create_dir_all(repo.join(".maw")).expect("maw dir");
        let restore = EnvVarRestore::capture("MAW_STATE_DIR");
        std::env::set_var("MAW_STATE_DIR", repo.join(".maw-test-state"));
        std::fs::write(repo.join(".maw/l1-oracle"), "01-gale\n").expect("l1 oracle");
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
        restore
    }

    #[test]
    fn pr_parse_flags_and_guard_option_injection() {
        let parsed = pr_parse_args(&pr_strings(&["codex", "--title", "Title", "--body=Body", "--show-current"])).expect("parse");
        assert_eq!(parsed.window.as_deref(), Some("codex"));
        assert_eq!(parsed.title.as_deref(), Some("Title"));
        assert_eq!(parsed.body.as_deref(), Some("Body"));
        assert!(parsed.show_current);
        assert!(pr_parse_args(&pr_strings(&["reconcile"])).expect("reconcile verb").reconcile);
        assert!(pr_parse_args(&pr_strings(&["--reconcile"])).expect("reconcile flag").reconcile);
        let quiet = pr_parse_args(&pr_strings(&["reconcile", "--quiet"])).expect("quiet reconcile");
        assert!(quiet.reconcile);
        assert!(quiet.quiet);
        assert!(pr_parse_args(&pr_strings(&["--quiet"])).is_err());
        assert!(pr_parse_args(&pr_strings(&["reconcile", "--title", "Title"])).is_err());
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
        let _state = pr_write_delivery(&repo, 140);
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
        assert_eq!(process.rerun_commands, vec!["cargo test".to_owned()]);
        assert_eq!(process.enqueued.len(), 1);
        assert_eq!(process.reconcile_quiet, vec![true]);
        assert!(!process.enqueued[0].notified);
        let events = std::fs::read_to_string(repo.join(".maw-test-state/l2-events.jsonl")).expect("l2 event queue");
        assert!(events.contains("[01-gale:"), "{events}");
        assert!(events.contains("READY issue #140 (standard/api)"), "{events}");
        let request = serde_json::from_str::<PrReviewRequest>(
            &std::fs::read_to_string(repo.join(".maw/l1-review-request.json")).expect("request"),
        )
        .expect("request json");
        assert!(!request.notified);
        assert_eq!(request.pr_number, 7);
        assert_eq!(request.status, "pending");
        assert_eq!(request.notified_at, None);
        assert_eq!(request.notifier, None);
    }

    #[test]
    fn pr_handoff_is_durable_without_sending_keys() {
        let _guard = env_test_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _restore = EnvVarRestore::capture("TMUX");
        std::env::set_var("TMUX", "/tmp/tmux,1,0");
        let repo = pr_temp_dir("unacknowledged-notify");
        let _state = pr_write_delivery(&repo, 55);
        let mut tmux = PrMockTmux { current_path: repo.display().to_string(), ..Default::default() };
        let mut process = PrMockProcess { branch: "agents/issue-55-l1-notify-ack".to_owned(), ..Default::default() };

        let output = pr_run(&[], &mut tmux, &mut process).expect("PR creation remains recoverable");

        assert!(output.contains("L1 handoff queued for the next hook drain"), "{output}");
        assert_eq!(process.enqueued.len(), 1);
        assert!(!process.enqueued[0].notified);
        assert_eq!(process.enqueued[0].status, "pending");
        let request = serde_json::from_str::<PrReviewRequest>(
            &std::fs::read_to_string(repo.join(".maw/l1-review-request.json")).expect("request"),
        )
        .expect("request json");
        assert!(!request.notified);
        assert_eq!(request.status, "pending");
        assert!(!repo.join(".maw/delivery-notified").exists());
    }

    #[test]
    fn pr_persists_l1_target_with_pending_review_for_reconciliation() {
        let _guard = env_test_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _restore = EnvVarRestore::capture("TMUX");
        std::env::set_var("TMUX", "/tmp/tmux,1,0");
        let repo = pr_temp_dir("durable-l1-target");
        let _state = pr_write_delivery(&repo, 55);
        std::fs::write(repo.join(".maw/l1-oracle"), "01-gale\n").expect("oracle target");
        std::fs::write(repo.join(".maw/l1-pane"), "%55\n").expect("pane target");
        let mut tmux = PrMockTmux { current_path: repo.display().to_string(), ..Default::default() };
        let mut process = PrMockProcess { branch: "agents/issue-55-l1-notify-ack".to_owned(), ..Default::default() };

        pr_run(&[], &mut tmux, &mut process).expect("create durable pending review");

        assert_eq!(process.enqueued[0].l1_oracle.as_deref(), Some("01-gale"));
        assert_eq!(process.enqueued[0].l1_pane.as_deref(), Some("%55"));
        let request = serde_json::from_str::<PrReviewRequest>(
            &std::fs::read_to_string(repo.join(".maw/l1-review-request.json")).expect("request"),
        )
        .expect("request json");
        assert_eq!(request.l1_oracle.as_deref(), Some("01-gale"));
        assert_eq!(request.l1_pane.as_deref(), Some("%55"));
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
    fn pr_without_pane_metadata_still_queues_durable_handoff() {
        let repo = pr_temp_dir("l1-pane-fallback");
        let _guard = env_test_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _restore = EnvVarRestore::capture("TMUX");
        std::env::set_var("TMUX", "/tmp/tmux,1,0");
        let _state = pr_write_delivery(&repo, 32);
        let mut tmux = PrMockTmux { current_path: repo.display().to_string(), ..Default::default() };
        let mut process = PrMockProcess { branch: "agents/issue-32-pr-durable-notification".to_owned(), ..PrMockProcess::default() };

        let output = pr_run(&[], &mut tmux, &mut process).expect("run");

        assert!(output.contains("L1 handoff queued for the next hook drain"), "{output}");
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
        let _state = pr_write_delivery(&repo, 42);
        let options = PrOptions {
            window: None,
            title: Some("Custom".to_owned()),
            body: Some("Body".to_owned()),
            show_current: false,
            reconcile: false,
            quiet: false,
        };
        let mut process = PrMockProcess::default();
        let plan = pr_build_plan(repo, "agents/issue-42-demo".to_owned(), "deachawatss/maw-rs".to_owned(), &options, &mut process).expect("plan");
        assert_eq!(plan.title, "Custom");
        assert!(plan.body.starts_with("Body\n\nCloses #42\nREQ: #42"));
        let error = pr_build_plan(std::path::PathBuf::from("/tmp"), String::new(), "deachawatss/maw-rs".to_owned(), &options, &mut process).expect_err("detached");
        assert!(error.contains("detached HEAD"));
    }

    #[test]
    fn pr_requires_valid_delivery_evidence_matching_branch_issue() {
        let repo = pr_temp_dir("delivery-required");
        let options = PrOptions { window: None, title: None, body: None, show_current: false, reconcile: false, quiet: false };
        let mut process = PrMockProcess::default();

        let missing = pr_build_plan(
            repo.clone(),
            "agents/issue-42-demo".to_owned(),
            "deachawatss/maw-rs".to_owned(),
            &options,
            &mut process,
        )
        .expect_err("missing delivery blocked");
        assert!(missing.contains(".maw/delivery.json"), "{missing}");

        let _state = pr_write_delivery(&repo, 41);
        let mismatch = pr_build_plan(
            repo.clone(),
            "agents/issue-42-demo".to_owned(),
            "deachawatss/maw-rs".to_owned(),
            &options,
            &mut process,
        )
        .expect_err("mismatched issue blocked");
        assert!(mismatch.contains("delivery issue 41 does not match branch issue 42"), "{mismatch}");

        let _state = pr_write_delivery(&repo, 42);
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
            &mut process,
        )
        .expect_err("invalid engine blocked");
        assert!(invalid_engine.contains("invalid delivery engine claude"), "{invalid_engine}");
    }

    #[test]
    fn pr_blocks_delivery_command_claimed_pass_when_rerun_fails() {
        let _guard = env_test_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _restore = EnvVarRestore::capture("TMUX");
        std::env::set_var("TMUX", "/tmp/tmux,1,0");
        let repo = pr_temp_dir("delivery-rerun-failure");
        let _state = pr_write_delivery(&repo, 53);
        let path = repo.join(".maw/delivery.json");
        let mut delivery: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("delivery body")).expect("delivery json");
        delivery["verification"]["commands"][0]["command"] = "exit 7".into();
        std::fs::write(&path, serde_json::to_string_pretty(&delivery).expect("render delivery")).expect("delivery");
        let mut tmux = PrMockTmux { current_path: repo.display().to_string(), ..Default::default() };
        let mut process = PrMockProcess {
            branch: "agents/issue-53-delivery-rerun".to_owned(),
            rerun_outcomes: [PrDeliveryCommandOutcome::Exited(Some(7))].into(),
            ..Default::default()
        };

        let error = pr_run(&[], &mut tmux, &mut process).expect_err("failed rerun blocks PR");

        assert!(error.contains("exit 7"), "{error}");
        assert!(error.contains("exit code 7"), "{error}");
        assert_eq!(process.rerun_commands, ["exit 7"]);
        assert!(process.created.is_empty());
    }

    #[test]
    fn pr_reruns_detailed_pass_and_surfaces_unrun_skip_in_handoff() {
        let _guard = env_test_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _restore = EnvVarRestore::capture("TMUX");
        std::env::set_var("TMUX", "/tmp/tmux,1,0");
        let repo = pr_temp_dir("delivery-skip");
        let _state = pr_write_delivery(&repo, 53);
        let path = repo.join(".maw/delivery.json");
        let mut delivery: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("delivery body")).expect("delivery json");
        delivery["verification"]["commands"] = serde_json::json!([
            {"command": "cargo test -p maw-cli", "result": "pass: focused CLI suite"},
            {"command": "gh pr view", "result": "skip: deferred to PR review"}
        ]);
        std::fs::write(&path, serde_json::to_string_pretty(&delivery).expect("render delivery")).expect("delivery");
        let mut tmux = PrMockTmux { current_path: repo.display().to_string(), ..Default::default() };
        let mut process = PrMockProcess { branch: "agents/issue-53-delivery-skip".to_owned(), ..Default::default() };

        pr_run(&[], &mut tmux, &mut process).expect("green verification creates PR");

        assert_eq!(process.rerun_commands, vec!["cargo test -p maw-cli".to_owned()]);
        assert!(process.created[0].body.contains("skip: deferred to PR review (not re-run)"));
    }

    #[test]
    fn pr_blocks_delivery_command_when_rerun_times_out() {
        let _guard = env_test_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _restore = EnvVarRestore::capture("TMUX");
        std::env::set_var("TMUX", "/tmp/tmux,1,0");
        let repo = pr_temp_dir("delivery-timeout");
        let _state = pr_write_delivery(&repo, 53);
        let mut tmux = PrMockTmux { current_path: repo.display().to_string(), ..Default::default() };
        let mut process = PrMockProcess {
            branch: "agents/issue-53-delivery-timeout".to_owned(),
            rerun_outcomes: [PrDeliveryCommandOutcome::TimedOut { timeout_ms: 100 }].into(),
            ..Default::default()
        };

        let error = pr_run(&[], &mut tmux, &mut process).expect_err("timed out rerun blocks PR");

        assert!(error.contains("cargo test"), "{error}");
        assert!(error.contains("timed out after 100ms"), "{error}");
        assert!(process.created.is_empty());
    }

    #[test]
    fn pr_native_delivery_rerun_reports_exit_code_and_timeout() {
        let _guard = env_test_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _restore = EnvVarRestore::capture(PR_DELIVERY_COMMAND_TIMEOUT_ENV);
        let repo = pr_temp_dir("delivery-native-rerun");
        let mut process = PrNativeProcess;

        assert_eq!(
            process.pr_rerun_delivery_command(&repo, "exit 7").expect("run exit"),
            PrDeliveryCommandOutcome::Exited(Some(7))
        );

        std::env::set_var(PR_DELIVERY_COMMAND_TIMEOUT_ENV, "100");
        assert_eq!(
            process.pr_rerun_delivery_command(&repo, "sleep 1").expect("run timeout"),
            PrDeliveryCommandOutcome::TimedOut { timeout_ms: 100 }
        );
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
            l1_oracle: Some("01-gale".to_owned()),
            l1_pane: None,
            reconcile_attempts: 0,
            last_reconcile_error: None,
        };

        pr_enqueue_global_review(&request).expect("enqueue pending");
        request.l1_pane = Some("%7".to_owned());
        pr_enqueue_global_review(&request).expect("upsert pending");

        let rows = std::fs::read_to_string(state.join("pr-queue.jsonl")).expect("queue");
        assert_eq!(rows.lines().count(), 1);
        assert_eq!(serde_json::from_str::<PrReviewRequest>(rows.trim()).expect("row"), request);
    }

    #[test]
    fn pr_reconcile_open_review_retains_durable_row_and_deduplicates_queue() {
        let _guard = env_test_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _restore = EnvVarRestore::capture("MAW_STATE_DIR");
        let state = pr_temp_dir("reconcile-open");
        std::env::set_var("MAW_STATE_DIR", &state);
        let request = PrReviewRequest {
            version: 1,
            pr_url: "https://github.com/acme/demo/pull/55".to_owned(),
            pr_number: 55,
            repo: "acme/demo".to_owned(),
            branch: "agents/issue-55-l1-notify-ack".to_owned(),
            status: "pending".to_owned(),
            notified: false,
            notified_at: None,
            notifier: None,
            l1_oracle: Some("01-gale".to_owned()),
            l1_pane: None,
            reconcile_attempts: 0,
            last_reconcile_error: None,
        };
        let row = pr_render_queue_row(&request).expect("row");
        std::fs::write(state.join("pr-queue.jsonl"), format!("{row}\n{row}\n")).expect("queue duplicates");
        let mut tmux = PrMockTmux::default();
        let mut process = PrMockProcess::default();

        let output = pr_run(&pr_strings(&["reconcile"]), &mut tmux, &mut process).expect("reconcile open PR");

        assert!(output.contains("PR #55 is still open; durable handoff retained"), "{output}");
        let rows = std::fs::read_to_string(state.join("pr-queue.jsonl")).expect("queue");
        assert_eq!(rows.lines().count(), 1);
        assert_eq!(serde_json::from_str::<PrReviewRequest>(rows.trim()).expect("row"), request);
        assert!(!state.join("pr-queue.jsonl.archived").exists());
    }

    #[test]
    fn pr_reconcile_open_review_never_requires_live_pane_delivery() {
        let _guard = env_test_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _restore = EnvVarRestore::capture("MAW_STATE_DIR");
        let state = pr_temp_dir("reconcile-open-busy");
        std::env::set_var("MAW_STATE_DIR", &state);
        let request = PrReviewRequest {
            version: 1,
            pr_url: "https://github.com/acme/demo/pull/57".to_owned(),
            pr_number: 57,
            repo: "acme/demo".to_owned(),
            branch: "agents/issue-55-l1-notify-ack".to_owned(),
            status: "pending".to_owned(),
            notified: false,
            notified_at: None,
            notifier: None,
            l1_oracle: Some("01-gale".to_owned()),
            l1_pane: None,
            reconcile_attempts: 0,
            last_reconcile_error: None,
        };
        pr_enqueue_global_review(&request).expect("enqueue");
        let mut tmux = PrMockTmux::default();
        let mut process = PrMockProcess::default();

        let output = pr_run(&pr_strings(&["reconcile"]), &mut tmux, &mut process).expect("durable reconciliation");

        assert!(output.contains("PR #57 is still open; durable handoff retained"), "{output}");
        let rows = std::fs::read_to_string(state.join("pr-queue.jsonl")).expect("queue");
        assert_eq!(serde_json::from_str::<PrReviewRequest>(rows.trim()).expect("pending row"), request);
        assert!(!state.join("pr-queue.jsonl.archived").exists());
    }

    #[test]
    fn pr_reconcile_archives_notified_merged_review_and_deduplicates_by_repo_and_number() {
        let _guard = env_test_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _restore = EnvVarRestore::capture("MAW_STATE_DIR");
        let state = pr_temp_dir("reconcile-notified-merged");
        std::env::set_var("MAW_STATE_DIR", &state);
        let request = PrReviewRequest {
            version: 1,
            pr_url: "https://github.com/acme/demo/pull/58".to_owned(),
            pr_number: 58,
            repo: "acme/demo".to_owned(),
            branch: "agents/issue-58-notified".to_owned(),
            status: "notified".to_owned(),
            notified: true,
            notified_at: Some("2026-07-14T00:00:00Z".to_owned()),
            notifier: Some("maw-pr".to_owned()),
            l1_oracle: Some("01-gale".to_owned()),
            l1_pane: Some("%58".to_owned()),
            reconcile_attempts: 0,
            last_reconcile_error: None,
        };
        let mut duplicate = request.clone();
        duplicate.pr_url = "https://legacy.example.invalid/acme/demo/pull/58".to_owned();
        let row = pr_render_queue_row(&request).expect("notified row");
        let duplicate_row = pr_render_queue_row(&duplicate).expect("duplicate row");
        std::fs::write(state.join("pr-queue.jsonl"), format!("{row}\n{duplicate_row}\n"))
            .expect("queue stale notified duplicates");
        let mut tmux = PrMockTmux::default();
        let mut process = PrMockProcess {
            review_state_results: [Ok(PrGithubState::Merged)].into(),
            ..PrMockProcess::default()
        };

        let output = pr_run(&pr_strings(&["reconcile"]), &mut tmux, &mut process)
            .expect("reconcile stale notified PR");

        assert!(output.contains("PR #58 merged; archiving queued handoff"), "{output}");
        assert!(process.review_state_results.is_empty(), "duplicate must not trigger a second GitHub check");
        assert!(std::fs::read_to_string(state.join("pr-queue.jsonl")).expect("queue").is_empty());
        let archived = std::fs::read_to_string(state.join("pr-queue.jsonl.archived")).expect("archive");
        assert_eq!(archived.lines().count(), 1);
        let archived_request = serde_json::from_str::<PrReviewRequest>(archived.trim()).expect("archive row");
        assert_eq!(archived_request.status, "merged");
        assert!(!archived_request.notified);
        assert_eq!(archived_request.repo, request.repo);
        assert_eq!(archived_request.pr_number, request.pr_number);
    }

    #[test]
    fn pr_reconcile_quiet_archives_merged_review_without_resurfacing() {
        let _guard = env_test_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _restore = EnvVarRestore::capture("MAW_STATE_DIR");
        let state = pr_temp_dir("reconcile-merged");
        std::env::set_var("MAW_STATE_DIR", &state);
        let request = PrReviewRequest {
            version: 1,
            pr_url: "https://github.com/acme/demo/pull/55".to_owned(),
            pr_number: 55,
            repo: "acme/demo".to_owned(),
            branch: "agents/issue-55-l1-notify-ack".to_owned(),
            status: "pending".to_owned(),
            notified: false,
            notified_at: None,
            notifier: None,
            l1_oracle: Some("01-gale".to_owned()),
            l1_pane: None,
            reconcile_attempts: 0,
            last_reconcile_error: None,
        };
        pr_enqueue_global_review(&request).expect("enqueue");
        let mut tmux = PrMockTmux::default();
        let mut process = PrMockProcess {
            review_state_results: [Ok(PrGithubState::Merged)].into(),
            ..PrMockProcess::default()
        };

        let output = pr_run(&pr_strings(&["--reconcile", "--quiet"]), &mut tmux, &mut process).expect("quiet reconcile merged PR");

        assert!(output.is_empty(), "{output}");
        assert!(std::fs::read_to_string(state.join("pr-queue.jsonl")).expect("queue").is_empty());
        let archived = std::fs::read_to_string(state.join("pr-queue.jsonl.archived")).expect("archive");
        let archived_request = serde_json::from_str::<PrReviewRequest>(archived.trim()).expect("archive row");
        assert_eq!(archived_request.status, "merged");
        assert!(!archived_request.notified);
        assert_eq!(archived_request.pr_url, request.pr_url);
    }

    #[test]
    fn pr_reconcile_closed_review_archives_without_resurfacing() {
        let _guard = env_test_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _restore = EnvVarRestore::capture("MAW_STATE_DIR");
        let state = pr_temp_dir("reconcile-closed");
        std::env::set_var("MAW_STATE_DIR", &state);
        let request = PrReviewRequest {
            version: 1,
            pr_url: "https://github.com/acme/demo/pull/56".to_owned(),
            pr_number: 56,
            repo: "acme/demo".to_owned(),
            branch: "agents/issue-55-l1-notify-ack".to_owned(),
            status: "pending".to_owned(),
            notified: false,
            notified_at: None,
            notifier: None,
            l1_oracle: Some("01-gale".to_owned()),
            l1_pane: None,
            reconcile_attempts: 0,
            last_reconcile_error: None,
        };
        pr_enqueue_global_review(&request).expect("enqueue");
        let mut tmux = PrMockTmux::default();
        let mut process = PrMockProcess {
            review_state_results: [Ok(PrGithubState::Closed)].into(),
            ..PrMockProcess::default()
        };

        let output = pr_run(&pr_strings(&["reconcile"]), &mut tmux, &mut process).expect("reconcile closed PR");

        assert!(output.contains("PR #56 closed; archiving queued handoff"), "{output}");
        let archived = std::fs::read_to_string(state.join("pr-queue.jsonl.archived")).expect("archive");
        assert_eq!(serde_json::from_str::<PrReviewRequest>(archived.trim()).expect("archive row").status, "closed");
    }

    #[test]
    fn pr_targets_origin_fork_main_and_rejects_soul_brews_upstream() {
        let _guard = env_test_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _restore = EnvVarRestore::capture("TMUX");
        std::env::set_var("TMUX", "/tmp/tmux,1,0");
        let repo = pr_temp_dir("fork-target");
        let _state = pr_write_delivery(&repo, 17);
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
