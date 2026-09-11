const DISPATCH_150: &[DispatcherEntry] = &[
    DispatcherEntry { command: "update", handler: Handler::Sync(update_run_command) },
    DispatcherEntry { command: "upgrade", handler: Handler::Sync(upgrade_run_command) },
];

const UPDATE_USAGE: &str = "usage: maw update [--dry-run]\n\n  Rebuild maw-rs from this checkout and put the new build everywhere it runs.\n\n  Four steps, in order. A failure stops the steps after it:\n    build     cargo build --release --bin maw-rs -j 4, under flock, in this checkout\n    install   copy the artifact over ~/.local/bin/maw-rs, keeping a timestamped backup\n    restart   pm2 restart maw-serve, when pm2 has that process\n    verify    maw doctor, read from the binary just installed — it reports whether the\n              daemon and the installed binary now agree, and it sets the exit code\n\n  Flags:\n    --dry-run     print the four steps and perform none of them\n    --yes, -y     accepted for compatibility; this command never prompts\n    --help, -h    show this message and exit (no side effects)\n\n  \u{26a0} The release build is expensive: about 7 minutes and 2 GB of target directory\n    from cold. Do not run it casually on a box with little disk.\n\n  `maw upgrade` is an alias for this command.";

const UPDATE_ALLOWED_FLAGS: &[&str] = &["--dry-run", "--yes", "-y", "--help", "-h"];
const UPDATE_BINARY: &str = "maw-rs";
const UPDATE_DAEMON_PROCESS: &str = "maw-serve";
/// `AGENTS.md`: run scoped *and* locked *and* capped, never two out of three. `--bin` is
/// the scope and this is the cap — a single locked, scoped release build saturated the box
/// on 2026-07-28 without it.
///
/// The lock is `flock`, not Cargo's. Measured 2026-09-11: two `--release` builds on the
/// shared `target-dir` do serialise on Cargo's own build-directory lock, but a `--release`
/// build and a dev-profile `cargo test` run side by side without blocking. Cross-profile
/// contention is the case the three incidents describe, so the external lock stays.
const UPDATE_BUILD_JOBS: &str = "4";
const UPDATE_BUILD_LOCK: &str = "flock";

#[derive(Debug, Clone, PartialEq, Eq)]
struct UpdateRequest150 { command: &'static str, help: bool, dry_run: bool }

fn update_run_command(argv: &[String]) -> CliOutput { update_run_command_in("update", argv) }

fn upgrade_run_command(argv: &[String]) -> CliOutput { upgrade_run_command_in(argv) }

fn upgrade_run_command_in(argv: &[String]) -> CliOutput { update_run_command_in("upgrade", argv) }

fn update_run_command_in(command: &'static str, argv: &[String]) -> CliOutput {
    match update_parse_request(command, argv) {
        Ok(request) if request.help => update_output(0, format!("{UPDATE_USAGE}\n"), String::new()),
        Ok(request) => update_release(&request),
        Err(message) => update_output(1, String::new(), format!("\u{1b}[31merror\u{1b}[0m: {message}\n")),
    }
}

fn update_parse_request(command: &'static str, argv: &[String]) -> Result<UpdateRequest150, String> {
    for arg in argv {
        if arg.chars().any(|ch| ch == '\0' || ch.is_control()) {
            return Err("arguments must not contain NUL or control characters".to_owned());
        }
        if !UPDATE_ALLOWED_FLAGS.contains(&arg.as_str()) {
            return Err(format!(
                "`maw {command}` takes no argument \"{arg}\" — it rebuilds the maw-rs checkout it runs in. Run `maw {command} --help` for usage."
            ));
        }
    }
    Ok(UpdateRequest150 {
        command,
        help: argv.iter().any(|arg| matches!(arg.as_str(), "--help" | "-h")),
        dry_run: argv.iter().any(|arg| arg == "--dry-run"),
    })
}

/// Build, install, restart and verify as one operation.
///
/// A rebuild does not reach the running daemon: `maw-serve` holds the old executable in
/// memory until something restarts it, and nothing did. That drift shipped three times —
/// 2026-06-12 in maw-js, 2026-07-28 with `maw-serve` twenty-one hours and eight merges
/// behind the installed binary, and 2026-09-10 during #207. `AGENTS.md` has carried the
/// rule in prose the whole time, so prose is not the instrument. The restart stops being
/// a step to remember by stopping being a separate step (#210).
fn update_release(request: &UpdateRequest150) -> CliOutput {
    let Some(checkout) = update_find_checkout() else {
        return update_step_error("build", &format!(
            "`maw {}` runs inside a maw-rs checkout — no crates/maw-cli/Cargo.toml in this directory or any parent",
            request.command
        ));
    };
    let Some(destination) = update_install_destination() else {
        return update_step_error("install", "HOME is unset, so the path the wrapper resolves is unknown");
    };
    if request.dry_run {
        return update_output(0, update_dry_run_plan(&checkout, &destination), String::new());
    }

    let target = match update_target_dir(&checkout) {
        Ok(path) => path,
        Err(message) => return update_step_error("build", &message),
    };
    let artifact = target.join("release").join(UPDATE_BINARY);
    let lock = update_lock_path(&target);
    let lock_arg = lock.to_string_lossy().into_owned();

    println!("build    cargo build --release --bin {UPDATE_BINARY} -j {UPDATE_BUILD_JOBS}, locked on {lock_arg}");
    if !update_spawn(
        UPDATE_BUILD_LOCK,
        &[&lock_arg, "cargo", "build", "--release", "--bin", UPDATE_BINARY, "-j", UPDATE_BUILD_JOBS],
        Some(&checkout),
    ) {
        return update_step_error("build", "cargo build failed — nothing was installed and no process was restarted");
    }
    if !artifact.is_file() {
        return update_step_error("install", &format!("the build produced no artifact at {} — refusing to install", artifact.display()));
    }
    let artifact_arg = artifact.to_string_lossy().into_owned();
    let destination_arg = destination.to_string_lossy().into_owned();

    println!("install  {artifact_arg} -> {destination_arg}");
    if destination.is_file() {
        let backup = update_backup_path(&destination);
        let backup_arg = backup.to_string_lossy().into_owned();
        if !update_spawn("install", &["-m755", &destination_arg, &backup_arg], None) {
            return update_step_error("install", &format!("could not copy {destination_arg} aside — refusing to overwrite it"));
        }
        println!("install  previous binary kept at {backup_arg}");
    }
    if !update_spawn("install", &["-m755", &artifact_arg, &destination_arg], None) {
        return update_step_error("install", &format!("could not install {artifact_arg} to {destination_arg}"));
    }

    if update_daemon_present() {
        println!("restart  pm2 restart {UPDATE_DAEMON_PROCESS}");
        if !update_spawn("pm2", &["restart", UPDATE_DAEMON_PROCESS], None) {
            return update_step_error("restart", &format!(
                "pm2 restart {UPDATE_DAEMON_PROCESS} failed — the new binary is installed but the daemon still holds the old one"
            ));
        }
    } else {
        println!("restart  skipped — pm2 has no {UPDATE_DAEMON_PROCESS} process");
    }

    update_verify(&destination)
}

fn update_dry_run_plan(checkout: &std::path::Path, destination: &std::path::Path) -> String {
    format!(
        "build    cargo build --release --bin {UPDATE_BINARY} -j {UPDATE_BUILD_JOBS}, locked, in {}\ninstall  the release artifact over {}, keeping a timestamped backup\nrestart  pm2 restart {UPDATE_DAEMON_PROCESS}, when pm2 has that process\nverify   {} doctor, reporting whether the daemon and the installed binary agree\ndry run: nothing was built, installed, restarted or verified\n",
        checkout.display(),
        destination.display(),
        destination.display()
    )
}

/// The nearest ancestor that is a maw-rs checkout.
fn update_find_checkout() -> Option<std::path::PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        if dir.join("crates").join("maw-cli").join("Cargo.toml").is_file() { return Some(dir); }
        if !dir.pop() { return None; }
    }
}

/// The path `scripts/maw-wrapper.sh` execs, and the only install destination.
fn update_install_destination() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|home| std::path::Path::new(&home).join(".local").join("bin").join(UPDATE_BINARY))
}

/// Ask Cargo where the release binary will land, before spending the build on it.
///
/// This repository ships its own `.cargo/config.toml` pointing `target-dir` at
/// `/tmp/maw-rs-target`, which overrides the machine's global setting. A path assembled
/// from the workspace root lands in an empty directory, and an empty directory reads as
/// "not built" — the failure #205 fixed in the deploy script.
fn update_target_dir(checkout: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let output = std::process::Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(checkout)
        .output()
        .map_err(|error| format!("could not run `cargo metadata`: {error}"))?;
    if !output.status.success() {
        return Err("`cargo metadata` failed, so the target directory is unknown — refusing to install".to_owned());
    }
    let parsed: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("could not parse `cargo metadata`: {error}"))?;
    let target = parsed
        .get("target_directory")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "`cargo metadata` reported no target_directory — refusing to install".to_owned())?;
    Ok(std::path::PathBuf::from(target))
}

/// The lock file `AGENTS.md` names, derived from the directory it guards.
fn update_lock_path(target: &std::path::Path) -> std::path::PathBuf {
    let mut name = target.as_os_str().to_owned();
    name.push(".lock");
    std::path::PathBuf::from(name)
}

fn update_backup_path(destination: &std::path::Path) -> std::path::PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or_default();
    let mut name = destination.as_os_str().to_owned();
    name.push(format!(".{stamp}.bak"));
    std::path::PathBuf::from(name)
}

/// Whether the process manager that owns the daemon has it. A box with no daemon is a
/// supported state, not a failure, so an unreachable `pm2` answers "no daemon" too.
fn update_daemon_present() -> bool {
    std::process::Command::new("pm2")
        .args(["describe", UPDATE_DAEMON_PROCESS])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Read the verdict from `doctor`, running the binary that was just installed.
///
/// The comparison has to come from the new binary: `MAW_RS_BUILD_VERSION` is a compile-time
/// constant, so this process would compare the *old* build against the restarted daemon and
/// call a correct release a failure. #190, #199 and #201 already settled what counts as drift
/// — a binary ahead of `main` and a `-dirty` stamp are both correct states — so this consumes
/// that check rather than writing a second one that could disagree with it.
fn update_verify(destination: &std::path::Path) -> CliOutput {
    let output = match std::process::Command::new(destination).args(["doctor", "--json"]).output() {
        Ok(output) => output,
        Err(error) => return update_step_error("verify", &format!("could not run {} doctor: {error}", destination.display())),
    };
    let Ok(parsed) = serde_json::from_slice::<serde_json::Value>(&output.stdout) else {
        return update_step_error("verify", &format!("{} doctor printed no readable JSON", destination.display()));
    };
    let check = parsed
        .get("checks")
        .and_then(serde_json::Value::as_array)
        .and_then(|checks| checks.iter().find(|check| check.get("name").and_then(serde_json::Value::as_str) == Some("version")));
    let Some(check) = check else {
        return update_step_error("verify", &format!("{} doctor reported no version check", destination.display()));
    };
    let message = check.get("message").and_then(serde_json::Value::as_str).unwrap_or("doctor reported no message");
    println!("verify   {message}");
    if check.get("ok").and_then(serde_json::Value::as_bool).unwrap_or(false) {
        update_output(0, String::new(), String::new())
    } else {
        update_output(1, String::new(), format!("\u{1b}[31merror\u{1b}[0m: verify: {message}\n"))
    }
}

fn update_spawn(program: &str, args: &[&str], cwd: Option<&std::path::Path>) -> bool {
    let mut command = std::process::Command::new(program);
    command.args(args);
    if let Some(dir) = cwd { command.current_dir(dir); }
    command.status().is_ok_and(|status| status.success())
}

fn update_step_error(step: &str, message: &str) -> CliOutput {
    update_output(1, String::new(), format!("\u{1b}[31merror\u{1b}[0m: {step}: {message}\n"))
}

fn update_output(code: i32, stdout: String, stderr: String) -> CliOutput { CliOutput { code, stdout, stderr } }

#[cfg(test)]
mod update_upgrade_tests150 {
    use super::*;

    fn update_args(values: &[&str]) -> Vec<String> { values.iter().map(|value| (*value).to_owned()).collect() }

    #[test]
    fn update_dispatch_registers_update_and_upgrade_native() {
        assert_eq!(dispatcher_status("update"), DispatchKind::Native);
        assert_eq!(dispatcher_status("upgrade"), DispatchKind::Native);
        assert_eq!(DISPATCH_150.len(), 2);
        assert_eq!(DISPATCH_150[0].command, "update");
        assert_eq!(DISPATCH_150[1].command, "upgrade");
    }

    #[test]
    fn update_parses_the_flags_it_accepts() {
        let parsed = update_parse_request("update", &update_args(&["--yes"])).expect("parse");
        assert!(!parsed.help);
        assert!(!parsed.dry_run);

        let parsed = update_parse_request("upgrade", &update_args(&["--dry-run"])).expect("parse");
        assert_eq!(parsed.command, "upgrade");
        assert!(parsed.dry_run);
    }

    #[test]
    fn update_rejects_a_ref_and_names_the_checkout_instead() {
        let out = update_run_command(&update_args(&["alpha"]));
        assert_eq!(out.code, 1);
        assert!(out.stdout.is_empty());
        assert!(out.stderr.contains("takes no argument \"alpha\""), "{}", out.stderr);
        assert!(out.stderr.contains("rebuilds the maw-rs checkout it runs in"), "{}", out.stderr);

        let control = update_run_command(&["main\nnext".to_owned()]);
        assert_eq!(control.code, 1);
        assert!(control.stderr.contains("control"), "{}", control.stderr);
    }

    #[test]
    fn update_help_names_the_four_steps_and_has_no_side_effects() {
        let out = update_run_command(&update_args(&["--help"]));
        assert_eq!(out.code, 0);
        assert!(out.stderr.is_empty());
        for step in ["build", "install", "restart", "verify"] {
            assert!(out.stdout.contains(step), "missing {step} in {}", out.stdout);
        }
        assert!(out.stdout.contains("expensive"), "{}", out.stdout);
    }

    #[test]
    fn upgrade_is_an_alias_for_update_not_a_second_path() {
        assert_eq!(
            upgrade_run_command(&update_args(&["--help"])).stdout,
            update_run_command(&update_args(&["--help"])).stdout
        );
    }
}
