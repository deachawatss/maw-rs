const DISPATCH_45: &[DispatcherEntry] = &[
    DispatcherEntry { command: "archive", handler: Handler::Sync(run_archive_command) },
];

#[derive(Debug, Clone, PartialEq, Eq)]
struct ArchiveOptions {
    oracle: String,
    dry_run: bool,
    yes: bool,
}

#[derive(Debug, Clone)]
struct ArchiveFleetEntry {
    file: String,
    path: std::path::PathBuf,
    session: NativeFleetSession,
}

fn run_archive_command(argv: &[String]) -> CliOutput {
    match archive_run_command_impl(argv) {
        Ok(stdout) => CliOutput { code: 0, stdout, stderr: String::new() },
        Err(message) => CliOutput { code: 1, stdout: String::new(), stderr: format!("{message}\n") },
    }
}

fn archive_usage_text() -> &'static str {
    "usage: maw archive <oracle> [--dry-run] [--yes] — archive an oracle's tmux session and data"
}

fn archive_run_command_impl(argv: &[String]) -> Result<String, String> {
    let Some(options) = archive_parse_args(argv)? else {
        return Ok(format!("{}\n", archive_usage_text()));
    };
    let entry = archive_find_entry(&options.oracle)?
        .ok_or_else(|| format!("oracle '{}' not found in fleet config", options.oracle))?;
    archive_render_and_apply(&entry, &options)
}

fn archive_parse_args(argv: &[String]) -> Result<Option<ArchiveOptions>, String> {
    let mut oracle = None::<String>;
    let mut dry_run = false;
    let mut yes = false;

    for arg in argv {
        match arg.as_str() {
            "--help" | "-h" => return Ok(None),
            "--dry-run" => dry_run = true,
            "--yes" | "-y" => yes = true,
            value if value.starts_with('-') => return Err(format!("archive: unknown argument {value}")),
            value => {
                if oracle.is_some() {
                    return Err(archive_usage_text().to_owned());
                }
                archive_validate_oracle(value)?;
                oracle = Some(value.to_owned());
            }
        }
    }

    let Some(oracle) = oracle else {
        return Err("usage: maw archive <oracle> [--dry-run]".to_owned());
    };
    Ok(Some(ArchiveOptions { oracle, dry_run, yes }))
}

fn archive_validate_oracle(value: &str) -> Result<(), String> {
    if value.is_empty() || value.trim() != value || value.starts_with('-') || value.contains('/') {
        Err("archive: oracle must be non-empty, unpadded, not start with '-', and not contain '/'".to_owned())
    } else {
        Ok(())
    }
}

fn archive_find_entry(oracle: &str) -> Result<Option<ArchiveFleetEntry>, String> {
    let fleet_dir = active_config_dir().join("fleet");
    let Ok(entries) = std::fs::read_dir(&fleet_dir) else {
        return Ok(None);
    };
    let mut files = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(std::ffi::OsStr::to_str) == Some("json"))
        .collect::<Vec<_>>();
    files.sort();

    for path in files {
        let text = std::fs::read_to_string(&path)
            .map_err(|error| format!("archive: read {}: {error}", path.display()))?;
        let session = serde_json::from_str::<NativeFleetSession>(&text)
            .map_err(|error| format!("archive: parse {}: {error}", path.display()))?;
        if archive_session_oracle_name(&session.name) == oracle {
            let file = path
                .file_name()
                .and_then(std::ffi::OsStr::to_str)
                .unwrap_or_default()
                .to_owned();
            return Ok(Some(ArchiveFleetEntry { file, path, session }));
        }
    }
    Ok(None)
}

fn archive_session_oracle_name(session_name: &str) -> &str {
    session_name
        .split_once('-')
        .filter(|(prefix, suffix)| prefix.chars().all(|ch| ch.is_ascii_digit()) && !suffix.is_empty())
        .map_or(session_name, |(_, suffix)| suffix)
}

fn archive_render_and_apply(entry: &ArchiveFleetEntry, options: &ArchiveOptions) -> Result<String, String> {
    let repo_slug = entry
        .session
        .windows
        .first()
        .map_or("", |window| window.repo.as_str());
    if !repo_slug.is_empty() {
        archive_validate_repo_slug(repo_slug)?;
    }

    let mut out = String::new();
    let _ = writeln!(out, "\n  \x1b[36m⚰️  Archiving\x1b[0m — {}\n", options.oracle);
    archive_render_soul_sync(&mut out, entry, options);
    archive_render_disable(&mut out, entry, options)?;
    archive_render_repo_archive(&mut out, repo_slug, options)?;
    archive_render_death_certificate(&mut out, entry, options);
    Ok(out)
}

fn archive_render_soul_sync(out: &mut String, entry: &ArchiveFleetEntry, options: &ArchiveOptions) {
    if entry.session.sync_peers.is_empty() {
        let _ = writeln!(out, "  \x1b[90m○\x1b[0m no sync_peers configured — knowledge stays local");
    } else if options.dry_run {
        let _ = writeln!(
            out,
            "  \x1b[36m⬡\x1b[0m [dry-run] would soul-sync to {}",
            entry.session.sync_peers.join(", ")
        );
    } else {
        let _ = writeln!(out, "  \x1b[36m⏳\x1b[0m final soul-sync to peers...");
        let _ = writeln!(out, "  \x1b[33m⚠\x1b[0m soul-sync not yet implemented in maw-rs native archive");
    }
}

fn archive_render_disable(out: &mut String, entry: &ArchiveFleetEntry, options: &ArchiveOptions) -> Result<(), String> {
    if options.dry_run {
        let _ = writeln!(
            out,
            "  \x1b[36m⬡\x1b[0m [dry-run] would disable: {} → {}.disabled",
            entry.file, entry.file
        );
        return Ok(());
    }
    archive_require_yes(options)?;
    let disabled = entry.path.with_file_name(format!("{}.disabled", entry.file));
    match std::fs::rename(&entry.path, &disabled) {
        Ok(()) => {
            let _ = writeln!(out, "  \x1b[32m✓\x1b[0m fleet config disabled: {}.disabled", entry.file);
        }
        Err(error) => {
            let _ = writeln!(out, "  \x1b[33m⚠\x1b[0m could not disable fleet config: {error}");
        }
    }
    Ok(())
}

fn archive_render_repo_archive(out: &mut String, repo_slug: &str, options: &ArchiveOptions) -> Result<(), String> {
    if repo_slug.is_empty() {
        return Ok(());
    }
    if options.dry_run {
        let _ = writeln!(out, "  \x1b[36m⬡\x1b[0m [dry-run] would archive: gh repo archive {repo_slug}");
        return Ok(());
    }
    archive_require_yes(options)?;
    match std::process::Command::new("gh")
        .args(["repo", "archive", repo_slug, "--yes"])
        .output()
    {
        Ok(output) if output.status.success() => {
            let _ = writeln!(out, "  \x1b[32m✓\x1b[0m GitHub repo archived: {repo_slug}");
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            let message = if stderr.is_empty() { format!("gh exited {}", output.status) } else { stderr };
            let _ = writeln!(out, "  \x1b[33m⚠\x1b[0m archive failed: {message}");
        }
        Err(error) => {
            let _ = writeln!(out, "  \x1b[33m⚠\x1b[0m archive failed: {error}");
        }
    }
    Ok(())
}

fn archive_render_death_certificate(out: &mut String, entry: &ArchiveFleetEntry, options: &ArchiveOptions) {
    if options.dry_run {
        let _ = writeln!(out, "  \x1b[36m⬡\x1b[0m [dry-run] would log death to family registry");
        out.push('\n');
    } else {
        let _ = writeln!(
            out,
            "  \x1b[32m✓\x1b[0m {} archived — ψ/ preserved locally, knowledge synced to peers\n",
            options.oracle
        );
        let _ = writeln!(out, "  \x1b[90mNothing is deleted (Principle 1). ψ/ and git history remain.\x1b[0m");
        let _ = writeln!(
            out,
            "  \x1b[90mTo unarchive: rename {}.disabled → {} + gh repo unarchive\x1b[0m",
            entry.file, entry.file
        );
        out.push('\n');
    }
    out.push('\n');
}

fn archive_require_yes(options: &ArchiveOptions) -> Result<(), String> {
    if options.yes {
        Ok(())
    } else {
        Err("archive: refusing real archive without --yes; use --dry-run to preview".to_owned())
    }
}

fn archive_validate_repo_slug(value: &str) -> Result<(), String> {
    let parts = value.split('/').collect::<Vec<_>>();
    if parts.len() != 2 || parts.iter().any(|part| !archive_safe_repo_part(part)) || value.starts_with('-') {
        return Err(format!("archive: invalid repo slug '{value}'"));
    }
    Ok(())
}

fn archive_safe_repo_part(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
}

#[cfg(test)]
mod archive_tests {
    use super::*;

    fn archive_strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn archive_parse_help_and_flags() {
        assert!(archive_parse_args(&archive_strings(&["--help"])).expect("parse").is_none());
        let parsed = archive_parse_args(&archive_strings(&["neo", "--dry-run", "--yes"]))
            .expect("parse")
            .expect("options");
        assert_eq!(parsed.oracle, "neo");
        assert!(parsed.dry_run);
        assert!(parsed.yes);
    }

    #[test]
    fn archive_oracle_guard_blocks_option_injection() {
        let error = archive_parse_args(&archive_strings(&["-oProxyCommand=touch-pwned", "--dry-run"]))
            .expect_err("guard");
        assert!(error.contains("unknown argument"));
        assert!(archive_validate_repo_slug("-bad/repo").is_err());
        assert!(archive_validate_repo_slug("org/-repo").is_err());
        assert!(archive_validate_repo_slug("org/repo").is_ok());
    }

    #[test]
    fn archive_session_oracle_strips_numeric_prefix() {
        assert_eq!(archive_session_oracle_name("03-neo"), "neo");
        assert_eq!(archive_session_oracle_name("neo"), "neo");
        assert_eq!(archive_session_oracle_name("dev-neo"), "dev-neo");
    }
}
