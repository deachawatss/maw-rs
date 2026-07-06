#![forbid(unsafe_code)]
#![allow(dead_code)]

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

pub(crate) fn sanitize_task_slug(task: &str) -> Result<String, String> {
    let slug = task
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .map(|ch| if ch == '/' { '-' } else { ch })
        .collect::<String>();
    if slug.is_empty() || slug.starts_with('.') {
        Err("workon: task slug must not be empty or start with '.'".to_owned())
    } else {
        Ok(slug)
    }
}

pub(crate) fn sanitize_fresh_worktree(
    repo_path: &Path,
    wt_path: &Path,
) -> Result<Vec<String>, String> {
    run_git_clean(wt_path)?;
    let mut cleaned = Vec::new();
    for relative in [
        ".maw/phase.json",
        ".maw/strategy.json",
        ".maw/solo-justified",
        ".maw/aggregate-verified",
        ".maw/done-pinged",
    ] {
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

pub(crate) fn prepare_engine(window_name: &str, cwd: &Path) -> Result<EngineResolution, String> {
    let config = load_config(cwd);
    let command = resolve_command(&config, window_name);
    let engine = detect_engine_name(&command);
    let warning = engine_warning(&config, &engine, repo_id_from_path(cwd).as_deref());
    let resolution = EngineResolution {
        engine,
        command,
        warning,
    };
    record_engine_choice(cwd, &resolution)?;
    Ok(resolution)
}

fn load_config(cwd: &Path) -> serde_json::Value {
    maw_xdg::load_merged_config_in_dir(&current_xdg_env(), cwd).config
}

fn resolve_command(config: &serde_json::Value, window_name: &str) -> String {
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

fn engine_warning(config: &serde_json::Value, engine: &str, repo: Option<&str>) -> Option<String> {
    if engine == "claude" || repo.is_some_and(|repo| repo_is_trusted(config, engine, repo)) {
        return None;
    }
    let repo = repo.unwrap_or("unknown repo");
    Some(format!(
        "non-Claude engine '{engine}' is not trusted for {repo}; Claude hook gates may not apply"
    ))
}

fn repo_is_trusted(config: &serde_json::Value, engine: &str, repo: &str) -> bool {
    [
        config.get("trustedRepos"),
        config.pointer("/workon/trustedRepos"),
        config.pointer(&format!("/engineTrustedRepos/{engine}")),
        config.pointer(&format!("/engines/{engine}/trustedRepos")),
    ]
    .into_iter()
    .any(|value| trust_array_matches(value, repo))
}

#[rustfmt::skip]
fn trust_array_matches(value: Option<&serde_json::Value>, repo: &str) -> bool {
    value.and_then(serde_json::Value::as_array).is_some_and(|items| items.iter().filter_map(serde_json::Value::as_str)
        .any(|pattern| pattern == "*" || pattern == repo || repo.ends_with(&format!("/{pattern}"))))
}

#[rustfmt::skip]
fn record_engine_choice(cwd: &Path, resolution: &EngineResolution) -> Result<bool, String> {
    let path = cwd.join(".maw/strategy.json");
    if !path.exists() { return Ok(false); }
    let body = std::fs::read_to_string(&path).map_err(|error| format!("workon: read {}: {error}", path.display()))?;
    let mut value = serde_json::from_str(&body).unwrap_or_else(|_| serde_json::json!({}));
    if !value.is_object() { value = serde_json::json!({}); }
    let object = value.as_object_mut().ok_or_else(|| "workon: strategy json must be an object".to_owned())?;
    object.insert("engine".to_owned(), resolution.engine.clone().into());
    object.insert("engineCommand".to_owned(), resolution.command.clone().into());
    object.insert("engineWarned".to_owned(), resolution.warning.is_some().into());
    if let Some(warning) = &resolution.warning { object.insert("engineWarning".to_owned(), warning.clone().into()); }
    let rendered = serde_json::to_string_pretty(&value).map_err(|error| format!("workon: render strategy json: {error}"))?;
    std::fs::write(&path, format!("{rendered}\n")).map_err(|error| format!("workon: write {}: {error}", path.display()))?;
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

#[rustfmt::skip]
fn current_xdg_env() -> maw_xdg::MawXdgEnv {
    const KEYS: &[&str] = &["MAW_HOME", "MAW_CONFIG_DIR", "MAW_XDG", "XDG_CONFIG_HOME", "XDG_STATE_HOME", "MAW_STATE_DIR", "XDG_DATA_HOME", "MAW_DATA_DIR", "XDG_CACHE_HOME", "MAW_CACHE_DIR", "MAW_TEST_MODE"];
    let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("."), PathBuf::from);
    let vars = KEYS.iter().filter_map(|key| std::env::var(key).ok().map(|value| ((*key).to_owned(), value)));
    maw_xdg::MawXdgEnv::with_vars(home, vars)
}
