#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Rescue uncommitted `ψ/` files from a worktree into the owning main checkout.
///
/// Existing destination files are never overwritten; collisions receive a timestamp suffix.
///
/// # Errors
///
/// Returns an error when Git inspection fails or when a rescue copy cannot be completed.
pub fn rescue_psi(worktree_path: &Path, fallback_main_path: &Path) -> Result<Vec<PathBuf>, String> {
    rescue_psi_with_mode(worktree_path, fallback_main_path, false)
}

/// List the `ψ/` files that a rescue would copy without modifying the main checkout.
pub fn preview_rescue_psi(
    worktree_path: &Path,
    fallback_main_path: &Path,
) -> Result<Vec<PathBuf>, String> {
    rescue_psi_with_mode(worktree_path, fallback_main_path, true)
}

fn rescue_psi_with_mode(
    worktree_path: &Path,
    fallback_main_path: &Path,
    dry_run: bool,
) -> Result<Vec<PathBuf>, String> {
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
    let tracked = tracked_psi_paths(worktree_path)?;
    let changed = status_psi_paths(&status);
    let mut sources = Vec::new();
    collect_psi_source(
        &worktree_path.join("ψ"),
        worktree_path,
        &tracked,
        &changed,
        &mut sources,
    )?;
    sources.sort();
    sources.dedup();
    if sources.is_empty() {
        return Ok(Vec::new());
    }
    let main_psi = main_path_from_git(worktree_path, fallback_main_path).join("ψ");
    let timestamp = unix_timestamp();
    let mut rescued = Vec::new();
    for source in sources {
        let destination = rescue_destination(worktree_path, &main_psi, &source, timestamp)?;
        if !dry_run {
            copy_without_overwrite(&source, &destination)?;
        }
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
    let main_path = absolute
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map_or_else(|| fallback.to_path_buf(), Path::to_path_buf);
    if main_path
        .components()
        .any(|component| component.as_os_str() == ".git")
    {
        fallback.to_path_buf()
    } else {
        main_path
    }
}

fn tracked_psi_paths(worktree_path: &Path) -> Result<BTreeSet<PathBuf>, String> {
    git(&[
        "-C".to_owned(),
        worktree_path.display().to_string(),
        "ls-files".to_owned(),
        "-z".to_owned(),
        "--".to_owned(),
        "ψ/".to_owned(),
    ])
    .map(|paths| {
        paths
            .split('\0')
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .collect()
    })
}

fn status_psi_paths(status: &str) -> BTreeSet<PathBuf> {
    status.lines().filter_map(status_psi_path).collect()
}

fn status_psi_path(line: &str) -> Option<PathBuf> {
    let path = line.get(3..)?.trim();
    let path = path
        .rsplit_once(" -> ")
        .map_or(path, |(_, destination)| destination.trim());
    let path = path.trim_matches('"');
    (path == "ψ" || path.starts_with("ψ/")).then(|| PathBuf::from(path))
}

fn collect_psi_source(
    path: &Path,
    worktree_path: &Path,
    tracked: &BTreeSet<PathBuf>,
    changed: &BTreeSet<PathBuf>,
    out: &mut Vec<PathBuf>,
) -> Result<(), String> {
    // lstat, never stat: a symlink must NOT be followed. A planted
    // `ψ/leak -> ~/.ssh/id_rsa` (or any link outside the worktree) would
    // otherwise be dereferenced and its target's contents copied into the
    // main repo ψ/ — data exfiltration. Skipping symlinks also removes the
    // symlink-cycle infinite-recursion risk. ψ/ holds regular memory files.
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return Ok(());
    };
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return Ok(());
    }
    if file_type.is_file() {
        let relative = path.strip_prefix(worktree_path).map_err(|_| {
            format!(
                "ψ rescue source escaped worktree '{}': {}",
                worktree_path.display(),
                path.display()
            )
        })?;
        if !tracked.contains(relative)
            || changed
                .iter()
                .any(|changed_path| relative.starts_with(changed_path))
        {
            out.push(path.to_path_buf());
        }
        return Ok(());
    }
    if !file_type.is_dir() {
        return Ok(());
    }
    let entries = std::fs::read_dir(path)
        .map_err(|error| format!("read ψ rescue dir '{}': {error}", path.display()))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("read ψ rescue entry '{}': {error}", path.display()))?;
        collect_psi_source(&entry.path(), worktree_path, tracked, changed, out)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    struct RescueTempRoot {
        path: PathBuf,
    }

    impl RescueTempRoot {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let sequence = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "maw-rs-done-rescue-{}-{sequence}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("temp root");
            Self { path }
        }
    }

    impl Drop for RescueTempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn rescue_git(repo: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .env("GIT_AUTHOR_NAME", "MAW rescue test")
            .env("GIT_AUTHOR_EMAIL", "rescue@example.invalid")
            .env("GIT_COMMITTER_NAME", "MAW rescue test")
            .env("GIT_COMMITTER_EMAIL", "rescue@example.invalid")
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn rescue_fixture() -> (RescueTempRoot, PathBuf, PathBuf, PathBuf) {
        let root = RescueTempRoot::new();
        let main = root.path.join("main");
        let worktree = root.path.join("worktree");
        let relative = PathBuf::from("ψ/memory/retrospectives/2026-07-27/rescue.md");
        std::fs::create_dir_all(&main).expect("main dir");
        rescue_git(&main, &["init", "-q"]);
        std::fs::write(main.join(".gitignore"), "ψ/\n").expect("ignore ψ");
        std::fs::write(main.join("README.md"), "rescue fixture\n").expect("seed readme");
        rescue_git(&main, &["add", ".gitignore", "README.md"]);
        rescue_git(&main, &["commit", "-m", "seed rescue fixture"]);
        let worktree_arg = worktree.to_str().expect("worktree UTF-8");
        rescue_git(
            &main,
            &["worktree", "add", "-b", "agents/rescue-psi", worktree_arg],
        );
        let note = worktree.join(&relative);
        std::fs::create_dir_all(note.parent().expect("note parent")).expect("note dir");
        std::fs::write(&note, "durable retrospective\n").expect("write note");
        (root, main, worktree, relative)
    }

    #[test]
    fn rescue_psi_copies_an_ignored_retro_from_a_worktree() {
        let (_root, main, worktree, relative) = rescue_fixture();

        let rescued = rescue_psi(&worktree, &main).expect("rescue ignored retro");

        let destination = main.join(&relative);
        assert_eq!(rescued, vec![destination.clone()]);
        assert_eq!(
            std::fs::read_to_string(destination).expect("rescued note"),
            "durable retrospective\n"
        );
    }
}
