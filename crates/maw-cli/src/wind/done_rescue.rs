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
    rescue_psi_with_mode(worktree_path, fallback_main_path, None, false)
}

/// Rescue into an explicit vault root instead of the worktree's own main checkout.
///
/// A delivery's notes belong to the oracle that dispatched it, not to whichever
/// repo the worktree happens to sit in — that vault is what Oracle v3 indexes, so
/// it is the only destination that makes a retro findable again. The caller
/// resolves the root (it needs fleet config, which this module deliberately does
/// not depend on) and is responsible for reporting which root it chose.
///
/// # Errors
///
/// Returns an error when Git inspection fails or when a rescue copy cannot be completed.
pub fn rescue_psi_into(
    worktree_path: &Path,
    destination_root: &Path,
    dry_run: bool,
) -> Result<Vec<PathBuf>, String> {
    if dry_run {
        match std::fs::symlink_metadata(worktree_path.join(".git")) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(format!(
                    "inspect ψ rescue Git metadata '{}': {error}",
                    worktree_path.display()
                ));
            }
        }
    }
    rescue_psi_with_mode(
        worktree_path,
        destination_root,
        Some(destination_root),
        dry_run,
    )
}

/// List the `ψ/` files that a rescue would copy without modifying the main checkout.
///
/// # Errors
///
/// Returns an error when Git metadata inspection fails, a required Git command fails
/// (including its stderr or exit status), or a source `ψ/` path cannot be inspected.
pub fn preview_rescue_psi(
    worktree_path: &Path,
    fallback_main_path: &Path,
) -> Result<Vec<PathBuf>, String> {
    match std::fs::symlink_metadata(worktree_path.join(".git")) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(format!(
                "inspect ψ rescue Git metadata '{}': {error}",
                worktree_path.display()
            ));
        }
    }
    rescue_psi_with_mode(worktree_path, fallback_main_path, None, true)
}

fn rescue_psi_with_mode(
    worktree_path: &Path,
    fallback_main_path: &Path,
    destination_root: Option<&Path>,
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
    let main_psi = destination_root
        .map_or_else(
            || main_path_from_git(worktree_path, fallback_main_path),
            Path::to_path_buf,
        )
        .join("ψ");
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
        Err(git_error_detail(
            String::from_utf8_lossy(&output.stderr).as_ref(),
            output.status.code(),
        ))
    }
}

fn git_error_detail(stderr: &str, code: Option<i32>) -> String {
    let stderr = stderr.trim();
    if stderr.is_empty() {
        code.map_or_else(
            || "git exited without a status code".to_owned(),
            |code| format!("git exited with status {code}"),
        )
    } else {
        stderr.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn git_errors_without_stderr_include_exit_status() {
        assert_eq!(git_error_detail("", Some(64)), "git exited with status 64");
    }

    fn temp_dir(label: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("maw-rs-rescue-{label}-{stamp}"));
        std::fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    fn write(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        std::fs::write(path, body).expect("write fixture file");
    }

    /// A worktree whose `.gitignore` swallows `ψ/`, holding one untracked retro.
    fn gitignored_psi_worktree(label: &str) -> PathBuf {
        let worktree = temp_dir(label);
        for args in [
            vec!["init", "--initial-branch=main"],
            vec!["config", "user.email", "gale@example.invalid"],
            vec!["config", "user.name", "Gale"],
        ] {
            let status = Command::new("git")
                .arg("-C")
                .arg(&worktree)
                .args(&args)
                .output()
                .expect("run git");
            assert!(status.status.success(), "git {args:?} failed");
        }
        write(&worktree.join(".gitignore"), "ψ/\n");
        write(
            &worktree.join("ψ/memory/retrospectives/2026-07/28/x.md"),
            "# retro that must survive retirement\n",
        );
        worktree
    }

    #[test]
    fn rescue_carries_out_a_gitignored_note_into_the_named_vault() {
        // The reported case in #175: `ψ/` is gitignored in the worktree, so the
        // note is invisible to `git status` — it must still be rescued, and it
        // must land in the vault the caller names rather than the worktree's own
        // main checkout.
        let worktree = gitignored_psi_worktree("ignored");
        let vault = temp_dir("vault");

        let rescued = rescue_psi_into(&worktree, &vault, false).expect("rescue");

        assert_eq!(rescued.len(), 1, "rescued: {rescued:?}");
        let landed = vault.join("ψ/memory/retrospectives/2026-07/28/x.md");
        assert!(landed.is_file(), "not rescued into the vault: {rescued:?}");
        assert_eq!(
            std::fs::read_to_string(&landed).expect("read rescued"),
            "# retro that must survive retirement\n"
        );
    }

    #[test]
    fn rescue_into_a_vault_never_overwrites_an_existing_note() {
        // AC 3: the collision rule must cover the new destination too, or a
        // second delivery silently clobbers the first one's retro.
        let worktree = gitignored_psi_worktree("collide");
        let vault = temp_dir("vault-collide");
        let occupied = vault.join("ψ/memory/retrospectives/2026-07/28/x.md");
        write(&occupied, "# the note already there\n");

        let rescued = rescue_psi_into(&worktree, &vault, false).expect("rescue");

        assert_eq!(rescued.len(), 1, "rescued: {rescued:?}");
        assert_eq!(
            std::fs::read_to_string(&occupied).expect("read incumbent"),
            "# the note already there\n",
            "the incumbent note was overwritten"
        );
        assert_ne!(rescued[0], occupied, "collision reused the occupied path");
        assert!(rescued[0].is_file(), "collision copy was not written");
    }

    #[test]
    fn rescue_dry_run_into_a_vault_writes_nothing() {
        let worktree = gitignored_psi_worktree("dry");
        let vault = temp_dir("vault-dry");

        let previewed = rescue_psi_into(&worktree, &vault, true).expect("preview");

        assert_eq!(previewed.len(), 1, "previewed: {previewed:?}");
        assert!(
            !vault.join("ψ").exists(),
            "dry run created {}",
            vault.join("ψ").display()
        );
    }
}
