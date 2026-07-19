#![forbid(unsafe_code)]

use std::{
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EngineResolution {
    pub(crate) engine: String,
    pub(crate) command: String,
    pub(crate) warning: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RepoLane {
    Product,
    Permissive,
    Lightweight,
}

pub(crate) const EPHEMERAL_MARKERS: &[&str] = &[
    ".maw/delivery.json",
    ".maw/l1-review-request.json",
    ".maw/delivery-notified",
    ".maw/l1-oracle",
    ".maw/pane-id",
    ".maw/auto-done-pinged",
    ".maw/phase.json",
    ".maw/req-line",
];

const LEGACY_MARKERS: &[&str] = &[
    ".maw/l1-pane",
    ".maw/strategy.json",
    ".maw/solo-justified",
    ".maw/spec-waived",
    ".maw/aggregate-verified",
    ".maw/done-pinged",
    ".maw/rrr-done",
];

const GITIGNORE_BLOCK_START: &str = "# >>> maw ephemeral markers (managed by maw-rs) >>>";
const GITIGNORE_BLOCK_END: &str = "# <<< maw ephemeral markers <<<";

pub(crate) fn repo_lane(repo_path: &Path, repo_name: &str) -> RepoLane {
    let marker = repo_path.join(".maw/lane");
    if let Ok(value) = std::fs::read_to_string(marker) {
        return match value.trim().to_ascii_lowercase().as_str() {
            "lightweight" => RepoLane::Lightweight,
            "permissive" => RepoLane::Permissive,
            _ => RepoLane::Product,
        };
    }

    let name = repo_name.to_ascii_lowercase();
    if name == "wind-framework"
        || name.starts_with("maw-")
        || name.starts_with("arra-")
        || name.ends_with("-oracle")
    {
        RepoLane::Lightweight
    } else {
        RepoLane::Product
    }
}

pub(crate) fn sanitize_fresh_worktree(
    repo_path: &Path,
    wt_path: &Path,
) -> Result<Vec<String>, String> {
    run_git_clean(wt_path)?;
    let mut cleaned = Vec::new();
    for relative in EPHEMERAL_MARKERS.iter().chain(LEGACY_MARKERS) {
        remove_stale_file(wt_path, relative, &mut cleaned)?;
    }
    for (label, path) in index_lock_candidates(wt_path) {
        if remove_file_if_present(&path)? {
            cleaned.push(label);
        }
    }
    if ensure_claude_md(repo_path, wt_path)? {
        cleaned.push("CLAUDE.md".to_owned());
    }
    Ok(cleaned)
}

pub(crate) fn prepare_engine(
    window_name: &str,
    cwd: &Path,
    explicit_engine: Option<&str>,
) -> Result<EngineResolution, String> {
    let config = load_config(cwd);
    let command = resolve_engine_command(&config, window_name, explicit_engine);
    let engine = detect_engine_name(&command);
    let logical_engine = explicit_engine
        .map(str::trim)
        .filter(|engine| !engine.is_empty());
    let warning = engine_warning(
        &config,
        &engine,
        logical_engine,
        repo_id_from_path(cwd).as_deref(),
    );
    let resolution = EngineResolution {
        engine,
        command,
        warning,
    };
    record_engine_choice(cwd, &resolution)?;
    Ok(resolution)
}

pub(crate) fn record_l1_oracle(cwd: &Path, oracle: &str) -> Result<bool, String> {
    let oracle = oracle.trim();
    if !valid_oracle_name(oracle) {
        return Ok(false);
    }
    let dir = cwd.join(".maw");
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("workon: create {}: {error}", dir.display()))?;
    let path = dir.join("l1-oracle");
    let tmp = dir.join(format!(".l1-oracle.{}.tmp", std::process::id()));
    std::fs::write(&tmp, format!("{oracle}\n"))
        .map_err(|error| format!("workon: write {}: {error}", tmp.display()))?;
    std::fs::rename(&tmp, &path)
        .map_err(|error| format!("workon: replace {}: {error}", path.display()))?;
    Ok(true)
}

fn valid_oracle_name(value: &str) -> bool {
    !value.is_empty()
        && value.trim() == value
        && !value.starts_with('-')
        && value
            .chars()
            .all(|ch| !ch.is_control() && !ch.is_whitespace())
}

fn load_config(cwd: &Path) -> serde_json::Value {
    maw_xdg::load_merged_config_in_dir(&current_xdg_env(), cwd).config
}

fn resolve_engine_command(
    config: &serde_json::Value,
    window_name: &str,
    explicit_engine: Option<&str>,
) -> String {
    if let Some(engine) = explicit_engine.filter(|value| !value.trim().is_empty()) {
        return config
            .get("commands")
            .and_then(serde_json::Value::as_object)
            .and_then(|commands| commands.get(engine))
            .and_then(serde_json::Value::as_str)
            .filter(|command| !command.trim().is_empty())
            .map_or_else(|| engine.to_owned(), ToOwned::to_owned);
    }
    let command = config
        .get("commands")
        .and_then(serde_json::Value::as_object)
        .and_then(|commands| {
            commands
                .get(window_name)
                .or_else(|| commands.get("default"))
                .and_then(serde_json::Value::as_str)
        })
        .filter(|command| !command.trim().is_empty());
    command.unwrap_or("claude").to_owned()
}

#[rustfmt::skip]
fn run_git_clean(wt_path: &Path) -> Result<(), String> {
    let output = Command::new("git").arg("-C").arg(wt_path).args(["clean", "-fd"]).output()
        .map_err(|error| format!("workon: failed to execute git clean: {error}"))?;
    if output.status.success() { return Ok(()); }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    Err(if stderr.is_empty() { "workon: git clean failed".to_owned() } else { format!("workon: git clean failed: {stderr}") })
}

fn remove_stale_file(
    wt_path: &Path,
    relative: &str,
    cleaned: &mut Vec<String>,
) -> Result<(), String> {
    if remove_file_if_present(&wt_path.join(relative))? {
        cleaned.push(relative.to_owned());
    }
    Ok(())
}

#[rustfmt::skip]
fn remove_file_if_present(path: &Path) -> Result<bool, String> {
    let Ok(metadata) = std::fs::symlink_metadata(path) else { return Ok(false); };
    if !metadata.file_type().is_file() && !metadata.file_type().is_symlink() {
        return Err(format!("workon: refused to remove non-file stale state: {}", path.display()));
    }
    std::fs::remove_file(path).map(|()| true).map_err(|error| format!("workon: remove {}: {error}", path.display()))
}

#[rustfmt::skip]
fn index_lock_candidates(wt_path: &Path) -> Vec<(String, PathBuf)> {
    let mut candidates = vec![(".git/index.lock".to_owned(), wt_path.join(".git/index.lock"))];
    let Ok(body) = std::fs::read_to_string(wt_path.join(".git")) else { return candidates; };
    let Some(git_dir) = body.trim().strip_prefix("gitdir:").map(str::trim) else { return candidates; };
    if !git_dir.is_empty() { candidates.push((".git/index.lock".to_owned(), path_from_worktree(wt_path, git_dir).join("index.lock"))); }
    candidates
}

fn path_from_worktree(wt_path: &Path, value: &str) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        wt_path.join(path)
    }
}

fn ensure_claude_md(repo_path: &Path, wt_path: &Path) -> Result<bool, String> {
    let (source, target) = (repo_path.join("CLAUDE.md"), wt_path.join("CLAUDE.md"));
    if target.exists() || !source.is_file() {
        return Ok(false);
    }
    std::fs::copy(&source, &target)
        .map(|_| true)
        .map_err(|error| format!("workon: copy {}: {error}", source.display()))
}

fn detect_engine_name(command: &str) -> String {
    command
        .split_whitespace()
        .find_map(engine_token)
        .unwrap_or_else(|| "unknown".to_owned())
}

fn engine_token(token: &str) -> Option<String> {
    let token = token.trim_matches(|ch| matches!(ch, '\'' | '"' | ';'));
    if token.is_empty() || matches!(token, "env" | "command") || is_env_assignment(token) {
        return None;
    }
    Path::new(token)
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .filter(|engine| !engine.is_empty())
        .map(str::to_owned)
}

fn is_env_assignment(token: &str) -> bool {
    token.split_once('=').is_some_and(|(key, _)| {
        !key.is_empty()
            && key
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    })
}

fn engine_warning(
    config: &serde_json::Value,
    binary_engine: &str,
    logical_engine: Option<&str>,
    repo: Option<&str>,
) -> Option<String> {
    if binary_engine == "claude"
        || repo.is_some_and(|repo| repo_is_trusted(config, logical_engine, binary_engine, repo))
    {
        return None;
    }
    let repo = repo.unwrap_or("unknown repo");
    Some(format!(
        "non-Claude engine '{binary_engine}' is not trusted for {repo}; Claude hook gates may not apply"
    ))
}

fn repo_is_trusted(
    config: &serde_json::Value,
    logical_engine: Option<&str>,
    binary_engine: &str,
    repo: &str,
) -> bool {
    [
        config.get("trustedRepos"),
        config.pointer("/workon/trustedRepos"),
    ]
    .into_iter()
    .any(|value| trust_array_matches(value, repo))
        // Logical names are the config contract; binary names remain an explicit
        // compatibility fallback. Both scoped paths use the same identity keys.
        || [logical_engine, Some(binary_engine)]
            .into_iter()
            .flatten()
            .any(|engine| {
                [
                    config.pointer(&format!("/engineTrustedRepos/{engine}")),
                    config.pointer(&format!("/engines/{engine}/trustedRepos")),
                ]
                .into_iter()
                .any(|value| trust_array_matches(value, repo))
            })
}

#[rustfmt::skip]
fn trust_array_matches(value: Option<&serde_json::Value>, repo: &str) -> bool {
    value.and_then(serde_json::Value::as_array).is_some_and(|items| items.iter().filter_map(serde_json::Value::as_str)
        .any(|pattern| pattern == "*" || pattern == repo || repo.ends_with(&format!("/{pattern}"))))
}

#[rustfmt::skip]
fn record_engine_choice(cwd: &Path, resolution: &EngineResolution) -> Result<bool, String> {
    let path = cwd.join(".maw/delivery.json");
    if !path.exists() && !cwd.join(".git").is_file() { return Ok(false); }
    let mut value: serde_json::Value = if path.exists() {
        let body = std::fs::read_to_string(&path).map_err(|error| format!("workon: read {}: {error}", path.display()))?;
        serde_json::from_str(&body).map_err(|error| format!("workon: invalid {}: {error}", path.display()))?
    } else {
        std::fs::create_dir_all(cwd.join(".maw"))
            .map_err(|error| format!("workon: create .maw: {error}"))?;
        serde_json::json!({ "version": 1 })
    };
    let object = value.as_object_mut().ok_or_else(|| "workon: delivery json must be an object".to_owned())?;
    object.insert("engine".to_owned(), resolution.engine.clone().into());
    object.insert("engineCommand".to_owned(), resolution.command.clone().into());
    object.insert("engineWarned".to_owned(), resolution.warning.is_some().into());
    if let Some(warning) = &resolution.warning { object.insert("engineWarning".to_owned(), warning.clone().into()); }
    let rendered = serde_json::to_string_pretty(&value).map_err(|error| format!("workon: render delivery json: {error}"))?;
    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&tmp, format!("{rendered}\n")).map_err(|error| format!("workon: write {}: {error}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|error| format!("workon: replace {}: {error}", path.display()))?;
    Ok(true)
}

fn repo_id_from_path(cwd: &Path) -> Option<String> {
    let rel = cwd.strip_prefix(ghq_root().join("github.com")).ok()?;
    let mut comps = rel.components();
    let org = comps.next()?.as_os_str().to_string_lossy();
    let repo = comps.next()?.as_os_str().to_string_lossy();
    Some(format!("{org}/{repo}"))
}

fn ghq_root() -> PathBuf {
    let Some(value) = std::env::var_os("GHQ_ROOT") else {
        return std::env::var_os("HOME").map_or_else(
            || PathBuf::from(".").join("Code"),
            |home| PathBuf::from(home).join("Code"),
        );
    };
    let mut path = PathBuf::from(value);
    if path.file_name().and_then(std::ffi::OsStr::to_str) == Some("github.com") {
        path.pop();
    }
    path
}

pub(crate) fn ensure_gitignore_ephemeral_block(root: &Path) -> Result<bool, String> {
    let gitignore = root.join(".gitignore");
    let existing = std::fs::read_to_string(&gitignore).unwrap_or_default();
    let mut block = String::new();
    block.push_str(GITIGNORE_BLOCK_START);
    block.push('\n');
    for marker in EPHEMERAL_MARKERS {
        block.push_str(marker);
        block.push('\n');
    }
    block.push_str(GITIGNORE_BLOCK_END);
    block.push('\n');

    let content = if let Some(start) = existing.find(GITIGNORE_BLOCK_START) {
        let tail = &existing[start..];
        let end_offset = tail.find(GITIGNORE_BLOCK_END).ok_or_else(|| {
            "workon: malformed managed .gitignore block (missing end marker)".to_owned()
        })?;
        let end = start + end_offset + GITIGNORE_BLOCK_END.len();
        let suffix = existing[end..]
            .strip_prefix('\n')
            .unwrap_or(&existing[end..]);
        format!("{}{}{}", &existing[..start], block, suffix)
    } else {
        let mut appended = existing.clone();
        if !appended.is_empty() && !appended.ends_with('\n') {
            appended.push('\n');
        }
        appended.push_str(&block);
        appended
    };
    if content == existing {
        return Ok(false);
    }
    std::fs::write(&gitignore, content)
        .map(|()| true)
        .map_err(|error| format!("workon: write .gitignore: {error}"))
}

#[rustfmt::skip]
fn current_xdg_env() -> maw_xdg::MawXdgEnv {
    const KEYS: &[&str] = &["MAW_HOME", "MAW_CONFIG_DIR", "MAW_XDG", "XDG_CONFIG_HOME", "XDG_STATE_HOME", "MAW_STATE_DIR", "XDG_DATA_HOME", "MAW_DATA_DIR", "XDG_CACHE_HOME", "MAW_CACHE_DIR", "MAW_TEST_MODE"];
    let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("."), PathBuf::from);
    let vars = KEYS.iter().filter_map(|key| std::env::var(key).ok().map(|value| ((*key).to_owned(), value)));
    maw_xdg::MawXdgEnv::with_vars(home, vars)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(label: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("maw-rs-workon-{label}-{stamp}"));
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    #[test]
    fn ephemeral_markers_list_is_complete() {
        let expected = [
            ".maw/delivery.json",
            ".maw/l1-review-request.json",
            ".maw/delivery-notified",
            ".maw/l1-oracle",
            ".maw/pane-id",
            ".maw/auto-done-pinged",
            ".maw/phase.json",
            ".maw/req-line",
        ];
        assert_eq!(EPHEMERAL_MARKERS, &expected);
    }

    #[test]
    fn lane_marker_overrides_infrastructure_name_fallback() {
        let repo = temp_dir("lane-marker");
        fs::create_dir_all(repo.join(".maw")).expect("lane dir");
        fs::write(repo.join(".maw/lane"), "product\n").expect("lane marker");

        assert_eq!(repo_lane(&repo, "maw-rs"), RepoLane::Product);
    }

    #[test]
    fn infrastructure_names_default_to_the_lightweight_lane() {
        let repo = temp_dir("lane-fallback");

        assert_eq!(repo_lane(&repo, "maw-rs"), RepoLane::Lightweight);
        assert_eq!(repo_lane(&repo, "arra-oracle-v3"), RepoLane::Lightweight);
        assert_eq!(repo_lane(&repo, "customer-api"), RepoLane::Product);
    }

    #[test]
    fn explicit_engine_selects_its_configured_command_instead_of_window_default() {
        let config = serde_json::json!({
            "commands": {
                "default": "claude",
                "demo-issue-42": "claude --model opus",
                "omx": "omx --xhigh"
            }
        });

        assert_eq!(
            resolve_engine_command(&config, "demo-issue-42", Some("omx")),
            "omx --xhigh"
        );
        assert_eq!(
            detect_engine_name(&resolve_engine_command(
                &config,
                "demo-issue-42",
                Some("omx")
            )),
            "omx"
        );
    }

    #[test]
    fn l1_oracle_name_is_strictly_validated() {
        assert!(valid_oracle_name("50-mawjs"));
        assert!(!valid_oracle_name(""));
        assert!(!valid_oracle_name(" 50-mawjs"));
        assert!(!valid_oracle_name("-oracle"));
        assert!(!valid_oracle_name("oracle\nnext"));
    }

    #[test]
    fn engine_choice_creates_delivery_skeleton_only_in_linked_worktree() {
        let worktree = temp_dir("engine-delivery");
        fs::write(
            worktree.join(".git"),
            "gitdir: /tmp/common/worktrees/demo\n",
        )
        .expect("git file");
        let resolution = EngineResolution {
            engine: "omx".to_owned(),
            command: "omx --xhigh".to_owned(),
            warning: None,
        };

        assert!(record_engine_choice(&worktree, &resolution).expect("record"));
        let value: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(worktree.join(".maw/delivery.json")).expect("delivery"),
        )
        .expect("json");
        assert_eq!(value["version"], 1);
        assert_eq!(value["engine"], "omx");
        assert_eq!(value["engineCommand"], "omx --xhigh");

        let main = temp_dir("engine-main");
        fs::create_dir_all(main.join(".git")).expect("git dir");
        assert!(!record_engine_choice(&main, &resolution).expect("skip main"));
        assert!(!main.join(".maw/delivery.json").exists());
    }

    #[test]
    fn remove_stale_file_removes_present_markers_and_preserves_absent() {
        let dir = temp_dir("stale-remove");
        let maw = dir.join(".maw");
        fs::create_dir_all(&maw).expect("maw dir");
        fs::create_dir_all(maw.join("plugins")).expect("plugins dir");
        fs::create_dir_all(maw.join("teams")).expect("teams dir");
        fs::create_dir_all(maw.join("fleet")).expect("fleet dir");
        fs::create_dir_all(maw.join("briefs")).expect("briefs dir");

        for marker in EPHEMERAL_MARKERS {
            fs::write(dir.join(marker), "stale").expect("write marker");
        }
        fs::write(maw.join("plugins/transcriber.json"), "{}").expect("plugin");
        fs::write(maw.join("teams/team-a.json"), "{}").expect("team");
        fs::write(maw.join("fleet/fleet.json"), "{}").expect("fleet");
        fs::write(maw.join("briefs/brief.md"), "# brief").expect("brief");

        let mut cleaned = Vec::new();
        for marker in EPHEMERAL_MARKERS {
            remove_stale_file(&dir, marker, &mut cleaned).unwrap();
        }

        for marker in EPHEMERAL_MARKERS {
            assert!(
                !dir.join(marker).exists(),
                "ephemeral marker should be removed: {marker}"
            );
        }

        assert!(
            maw.join("plugins/transcriber.json").exists(),
            "plugins preserved"
        );
        assert!(maw.join("teams/team-a.json").exists(), "teams preserved");
        assert!(maw.join("fleet/fleet.json").exists(), "fleet preserved");
        assert!(maw.join("briefs/brief.md").exists(), "briefs preserved");

        assert_eq!(cleaned.len(), EPHEMERAL_MARKERS.len());
        for marker in EPHEMERAL_MARKERS {
            assert!(
                cleaned.contains(&(*marker).to_owned()),
                "cleaned list should contain {marker}"
            );
        }
    }

    #[test]
    fn remove_stale_file_tolerates_missing_markers() {
        let dir = temp_dir("stale-missing");
        fs::create_dir_all(dir.join(".maw")).expect("maw dir");

        let mut cleaned = Vec::new();
        for marker in EPHEMERAL_MARKERS {
            remove_stale_file(&dir, marker, &mut cleaned).unwrap();
        }
        assert!(cleaned.is_empty());
    }

    #[test]
    fn gitignore_block_written_on_first_call() {
        let dir = temp_dir("gitignore-first");
        fs::write(dir.join(".gitignore"), "target/\n").expect("seed");

        let wrote = ensure_gitignore_ephemeral_block(&dir).unwrap();
        assert!(wrote);

        let content = fs::read_to_string(dir.join(".gitignore")).unwrap();
        assert!(content.contains(GITIGNORE_BLOCK_START));
        assert!(content.contains(GITIGNORE_BLOCK_END));
        for marker in EPHEMERAL_MARKERS {
            assert!(
                content.contains(marker),
                "gitignore should contain {marker}"
            );
        }
        assert!(
            content.starts_with("target/\n"),
            "original content preserved"
        );
    }

    #[test]
    fn gitignore_block_is_idempotent() {
        let dir = temp_dir("gitignore-idempotent");
        fs::write(dir.join(".gitignore"), "target/\n").expect("seed");

        ensure_gitignore_ephemeral_block(&dir).unwrap();
        let first = fs::read_to_string(dir.join(".gitignore")).unwrap();

        let wrote = ensure_gitignore_ephemeral_block(&dir).unwrap();
        assert!(!wrote, "second call should be a no-op");

        let second = fs::read_to_string(dir.join(".gitignore")).unwrap();
        assert_eq!(first, second, "content should not be duplicated");
    }

    #[test]
    fn gitignore_block_migrates_legacy_markers() {
        let dir = temp_dir("gitignore-migrate");
        fs::write(
            dir.join(".gitignore"),
            format!("target/\n{GITIGNORE_BLOCK_START}\n.maw/strategy.json\n{GITIGNORE_BLOCK_END}\nnotes/\n"),
        )
        .expect("seed");

        assert!(ensure_gitignore_ephemeral_block(&dir).expect("migrate"));
        let content = fs::read_to_string(dir.join(".gitignore")).expect("read");
        assert!(content.contains(".maw/delivery.json"));
        assert!(!content.contains(".maw/strategy.json"));
        assert!(content.ends_with("notes/\n"));
        assert!(!ensure_gitignore_ephemeral_block(&dir).expect("idempotent"));
    }

    #[test]
    fn gitignore_block_created_when_no_gitignore_exists() {
        let dir = temp_dir("gitignore-new");

        let wrote = ensure_gitignore_ephemeral_block(&dir).unwrap();
        assert!(wrote);

        let content = fs::read_to_string(dir.join(".gitignore")).unwrap();
        assert!(content.contains(GITIGNORE_BLOCK_START));
        assert!(content.contains(".maw/delivery.json"));
    }
}
