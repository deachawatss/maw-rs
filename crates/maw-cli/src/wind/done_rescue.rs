#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Rescue uncommitted `ψ/` files from a worktree into the owning main checkout.
///
/// Existing destination files are never overwritten; collisions receive a timestamp suffix.
///
/// # Errors
///
/// Returns an error when git status fails or when a rescue copy cannot be completed.
pub fn rescue_psi(worktree_path: &Path, fallback_main_path: &Path) -> Result<Vec<PathBuf>, String> {
    let status = git(&[
        "-C".to_owned(),
        worktree_path.display().to_string(),
        "-c".to_owned(),
        "core.quotePath=false".to_owned(),
        "status".to_owned(),
        "--porcelain".to_owned(),
        "--".to_owned(),
        "ψ/".to_owned(),
    ])?;
    let sources = uncommitted_psi_sources(worktree_path, &status)?;
    if sources.is_empty() {
        return Ok(Vec::new());
    }
    let main_psi = main_path_from_git(worktree_path, fallback_main_path).join("ψ");
    let timestamp = unix_timestamp();
    let mut rescued = Vec::new();
    for source in sources {
        let destination = rescue_destination(worktree_path, &main_psi, &source, timestamp)?;
        copy_without_overwrite(&source, &destination)?;
        rescued.push(destination);
    }
    Ok(rescued)
}

fn main_path_from_git(worktree_path: &Path, fallback: &Path) -> PathBuf {
    let common_dir = git(&[
        "-C".to_owned(),
        worktree_path.display().to_string(),
        "rev-parse".to_owned(),
        "--git-common-dir".to_owned(),
    ])
    .unwrap_or_default();
    let common_dir = common_dir.trim();
    if common_dir.is_empty() {
        return fallback.to_path_buf();
    }
    let path = PathBuf::from(common_dir);
    let absolute = if path.is_absolute() {
        path
    } else {
        worktree_path.join(path)
    };
    absolute
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map_or_else(|| fallback.to_path_buf(), Path::to_path_buf)
}

fn uncommitted_psi_sources(worktree_path: &Path, status: &str) -> Result<Vec<PathBuf>, String> {
    let mut sources = Vec::new();
    for relative in status.lines().filter_map(status_psi_path) {
        collect_psi_source(&worktree_path.join(relative), &mut sources)?;
    }
    sources.sort();
    sources.dedup();
    Ok(sources)
}

fn status_psi_path(line: &str) -> Option<PathBuf> {
    let path = line.get(3..)?.trim();
    let path = path
        .rsplit_once(" -> ")
        .map_or(path, |(_, destination)| destination.trim());
    let path = path.trim_matches('"');
    (path == "ψ" || path.starts_with("ψ/")).then(|| PathBuf::from(path))
}

fn collect_psi_source(path: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    if path.is_file() {
        out.push(path.to_path_buf());
        return Ok(());
    }
    if !path.is_dir() {
        return Ok(());
    }
    let entries = std::fs::read_dir(path)
        .map_err(|error| format!("read ψ rescue dir '{}': {error}", path.display()))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("read ψ rescue entry '{}': {error}", path.display()))?;
        collect_psi_source(&entry.path(), out)?;
    }
    Ok(())
}

fn rescue_destination(
    worktree_path: &Path,
    main_psi: &Path,
    source: &Path,
    timestamp: u64,
) -> Result<PathBuf, String> {
    let psi_root = worktree_path.join("ψ");
    let relative = source
        .strip_prefix(&psi_root)
        .map_err(|_| format!("ψ rescue source escaped ψ/: {}", source.display()))?;
    Ok(available_destination(&main_psi.join(relative), timestamp))
}

fn available_destination(path: &Path, timestamp: u64) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    for attempt in 0_u32..1000 {
        let candidate = collision_destination(path, timestamp, attempt);
        if !candidate.exists() {
            return candidate;
        }
    }
    collision_destination(path, timestamp, std::process::id())
}

fn collision_destination(path: &Path, timestamp: u64, attempt: u32) -> PathBuf {
    let suffix = if attempt == 0 {
        format!("-{timestamp}")
    } else {
        format!("-{timestamp}-{attempt}")
    };
    let file_stem = path
        .file_stem()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("psi");
    let file_name = path
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .map_or_else(
            || format!("{file_stem}{suffix}"),
            |extension| format!("{file_stem}{suffix}.{extension}"),
        );
    path.with_file_name(file_name)
}

fn copy_without_overwrite(source: &Path, destination: &Path) -> Result<(), String> {
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create ψ rescue dir '{}': {error}", parent.display()))?;
    }
    let mut input = std::fs::File::open(source)
        .map_err(|error| format!("open ψ rescue source '{}': {error}", source.display()))?;
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|error| {
            format!(
                "create ψ rescue destination '{}': {error}",
                destination.display()
            )
        })?;
    std::io::copy(&mut input, &mut output).map_err(|error| {
        format!(
            "copy ψ rescue '{}' -> '{}': {error}",
            source.display(),
            destination.display()
        )
    })?;
    Ok(())
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

fn git(args: &[String]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .output()
        .map_err(|error| format!("git failed: {error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}
