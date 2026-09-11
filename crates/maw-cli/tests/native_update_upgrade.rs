use maw_cli::{dispatcher_status, DispatchKind};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

/// A checkout root with a fake `PATH`, a fake `HOME`, and a marker log.
///
/// Every executable `maw update` reaches for — `cargo`, `install`, `pm2`, and the binary it
/// installs — is a script in `bin/` that appends its own argv to `marker.log`. The log is
/// what proves a step ran; stdout is what proves the command reported it. The real
/// `/usr/bin` stays on `PATH` behind `bin/` so those scripts can still use `cp` and `mkdir`.
struct UpdateFixture {
    root: PathBuf,
}

impl UpdateFixture {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).expect("clock").as_nanos();
        let root = std::env::temp_dir().join(format!("maw-rs-native-update-{label}-{}-{nonce}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("bin")).expect("bin");
        fs::create_dir_all(root.join("checkout/crates/maw-cli")).expect("checkout");
        fs::write(root.join("checkout/crates/maw-cli/Cargo.toml"), "[package]\nname = \"maw-cli\"\n").expect("crate manifest");
        Self { root }
    }

    fn path(&self, relative: &str) -> PathBuf { self.root.join(relative) }

    fn write_script(&self, name: &str, body: &str) {
        let path = self.root.join("bin").join(name);
        fs::write(&path, body).expect("script");
        let mut perms = fs::metadata(&path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).expect("chmod");
    }

    /// `cargo metadata` reports the target directory; `cargo build` puts a stand-in binary in
    /// it that answers `doctor --json` the way the verify step reads it.
    fn write_cargo(&self, build_exit: i32, version_ok: bool) {
        let target = self.path("target");
        let message = if version_ok {
            "daemon and installed binary both test-build"
        } else {
            "daemon on :3456 is running old-build but the installed binary is test-build — it has not been restarted since the rebuild"
        };
        self.write_script("cargo", &format!(
            "#!/bin/bash\n\
             echo \"cargo $*\" >> \"$MARKER_LOG\"\n\
             if [ \"$1\" = \"metadata\" ]; then echo '{{\"target_directory\":\"{target}\"}}'; exit 0; fi\n\
             if [ \"$1\" = \"build\" ]; then\n\
               if [ {build_exit} -ne 0 ]; then exit {build_exit}; fi\n\
               mkdir -p '{target}/release'\n\
               cat > '{target}/release/maw-rs' <<'INNER'\n\
#!/bin/bash\n\
echo \"installed-binary $*\" >> \"$MARKER_LOG\"\n\
echo '{{\"ok\":true,\"checks\":[{{\"name\":\"version\",\"ok\":{version_ok},\"severity\":\"info\",\"message\":\"{message}\"}}],\"comparison\":[]}}'\n\
INNER\n\
               chmod 755 '{target}/release/maw-rs'\n\
               exit 0\n\
             fi\n\
             exit 0\n",
            target = target.display()
        ));
    }

    /// The real `install(1)` contract: the last two arguments are source and destination.
    fn write_install(&self) {
        self.write_script(
            "install",
            "#!/bin/bash\n\
             echo \"install $*\" >> \"$MARKER_LOG\"\n\
             src=\"${@: -2:1}\"; dest=\"${@: -1}\"\n\
             mkdir -p \"$(dirname \"$dest\")\"\n\
             cp \"$src\" \"$dest\" && chmod 755 \"$dest\"\n",
        );
    }

    /// `flock <lockfile> <command...>` — logs, then runs the command it was handed, so the
    /// build marker still appears and a failing build still propagates its exit code.
    fn write_flock(&self) {
        self.write_script(
            "flock",
            "#!/bin/bash\n             echo \"flock $*\" >> \"$MARKER_LOG\"\n             exec \"${@:2}\"\n",
        );
    }

    fn write_pm2(&self, daemon_present: bool) {
        let describe_exit = i32::from(!daemon_present);
        self.write_script("pm2", &format!(
            "#!/bin/bash\n\
             echo \"pm2 $*\" >> \"$MARKER_LOG\"\n\
             if [ \"$1\" = \"describe\" ]; then exit {describe_exit}; fi\n\
             exit 0\n"
        ));
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_maw-rs"))
            .args(args)
            .current_dir(self.path("checkout"))
            .env_clear()
            .env("PATH", format!("{}:/usr/bin:/bin", self.path("bin").display()))
            .env("HOME", &self.root)
            .env("MARKER_LOG", self.path("marker.log"))
            .env("XDG_CONFIG_HOME", self.path("config"))
            .env("XDG_STATE_HOME", self.path("state"))
            .env("MAW_CONFIG_DIR", self.path("config/maw"))
            .env("CARGO_TERM_COLOR", "never")
            .output()
            .expect("run update")
    }

    fn markers(&self) -> Vec<String> {
        fs::read_to_string(self.path("marker.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn destination(&self) -> PathBuf { self.path(".local/bin/maw-rs") }
}

impl Drop for UpdateFixture {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.root); }
}

fn assert_markers_in_order(markers: &[String], expected: &[&str]) {
    let mut cursor = 0_usize;
    for needle in expected {
        let found = markers[cursor..].iter().position(|line| line.contains(needle));
        let Some(offset) = found else {
            panic!("missing marker {needle:?} after position {cursor} in {markers:#?}");
        };
        cursor += offset + 1;
    }
}

fn assert_no_marker(markers: &[String], needle: &str) {
    assert!(!markers.iter().any(|line| line.contains(needle)), "unexpected marker {needle:?} in {markers:#?}");
}

fn stdout_of(output: &Output) -> String { String::from_utf8_lossy(&output.stdout).into_owned() }

fn stderr_of(output: &Output) -> String { String::from_utf8_lossy(&output.stderr).into_owned() }

#[test]
fn update_builds_installs_restarts_and_verifies_in_that_order() {
    assert_eq!(dispatcher_status("update"), DispatchKind::Native);
    let fixture = UpdateFixture::new("full-release");
    fixture.write_cargo(0, true);
    fixture.write_flock();
    fixture.write_install();
    fixture.write_pm2(true);
    // A binary is already installed, so the previous one has to be kept.
    fs::create_dir_all(fixture.path(".local/bin")).expect("bin dir");
    fs::write(fixture.destination(), "#!/bin/sh\nexit 0\n").expect("previous binary");

    let output = fixture.run(&["update"]);
    let stdout = stdout_of(&output);
    assert_eq!(output.status.code(), Some(0), "stderr={}", stderr_of(&output));

    let markers = fixture.markers();
    assert_markers_in_order(&markers, &[
        "flock ",
        "cargo build --release --bin maw-rs -j 4",
        "install -m755",
        "install -m755",
        "pm2 restart maw-serve",
        "installed-binary doctor --json",
    ]);
    for step in ["build ", "install ", "restart ", "verify "] {
        assert!(stdout.contains(step), "missing {step:?} in {stdout}");
    }
    assert!(stdout.contains("daemon and installed binary both test-build"), "{stdout}");

    let backups = fs::read_dir(fixture.path(".local/bin"))
        .expect("bin dir")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".bak"))
        .count();
    assert_eq!(backups, 1, "the previous binary is kept beside the destination");
}

#[test]
fn update_skips_the_restart_when_pm2_has_no_daemon_and_still_verifies() {
    let fixture = UpdateFixture::new("no-daemon");
    fixture.write_cargo(0, true);
    fixture.write_flock();
    fixture.write_install();
    fixture.write_pm2(false);

    let output = fixture.run(&["update"]);
    let stdout = stdout_of(&output);
    assert_eq!(output.status.code(), Some(0), "no daemon is a supported state; stderr={}", stderr_of(&output));

    let markers = fixture.markers();
    assert_markers_in_order(&markers, &["cargo build", "install -m755", "installed-binary doctor --json"]);
    assert_no_marker(&markers, "pm2 restart");
    assert!(stdout.contains("restart  skipped"), "{stdout}");
    assert!(stdout.contains("verify   "), "{stdout}");
}

#[test]
fn update_stops_at_a_failed_build_without_installing_or_restarting() {
    let fixture = UpdateFixture::new("failed-build");
    fixture.write_cargo(1, true);
    fixture.write_flock();
    fixture.write_install();
    fixture.write_pm2(true);

    let output = fixture.run(&["update"]);
    assert_eq!(output.status.code(), Some(1));

    let markers = fixture.markers();
    assert_markers_in_order(&markers, &["cargo build"]);
    assert_no_marker(&markers, "install -m755");
    assert_no_marker(&markers, "pm2 restart");
    assert!(stderr_of(&output).contains("build: cargo build failed"), "{}", stderr_of(&output));
    assert!(!fixture.destination().exists(), "a failed build must not install anything");
}

#[test]
fn update_exits_non_zero_when_the_daemon_still_disagrees_after_the_restart() {
    let fixture = UpdateFixture::new("still-stale");
    fixture.write_cargo(0, false);
    fixture.write_flock();
    fixture.write_install();
    fixture.write_pm2(true);

    let output = fixture.run(&["update"]);
    assert_eq!(output.status.code(), Some(1));
    assert_markers_in_order(&fixture.markers(), &["pm2 restart maw-serve", "installed-binary doctor --json"]);
    assert!(stderr_of(&output).contains("verify:"), "{}", stderr_of(&output));
    assert!(stdout_of(&output).contains("it has not been restarted since the rebuild"), "{}", stdout_of(&output));
}

#[test]
fn upgrade_is_the_same_operation_under_the_other_name() {
    assert_eq!(dispatcher_status("upgrade"), DispatchKind::Native);
    let fixture = UpdateFixture::new("upgrade-alias");
    fixture.write_cargo(0, true);
    fixture.write_flock();
    fixture.write_install();
    fixture.write_pm2(true);

    let output = fixture.run(&["upgrade"]);
    assert_eq!(output.status.code(), Some(0), "stderr={}", stderr_of(&output));
    assert_markers_in_order(&fixture.markers(), &["cargo build", "install -m755", "pm2 restart maw-serve", "installed-binary doctor --json"]);
}

#[test]
fn update_dry_run_and_help_name_the_steps_and_touch_nothing() {
    let fixture = UpdateFixture::new("no-side-effects");
    fixture.write_cargo(0, true);
    fixture.write_flock();
    fixture.write_install();
    fixture.write_pm2(true);

    let help = fixture.run(&["update", "--help"]);
    assert_eq!(help.status.code(), Some(0));
    assert!(stdout_of(&help).contains("expensive"), "{}", stdout_of(&help));

    let dry = fixture.run(&["update", "--dry-run"]);
    assert_eq!(dry.status.code(), Some(0), "stderr={}", stderr_of(&dry));
    let stdout = stdout_of(&dry);
    for step in ["build ", "install ", "restart ", "verify "] {
        assert!(stdout.contains(step), "missing {step:?} in {stdout}");
    }
    assert!(stdout.contains("dry run: nothing was built"), "{stdout}");

    assert!(fixture.markers().is_empty(), "help and dry-run must invoke nothing: {:#?}", fixture.markers());
    assert!(!fixture.path("target").exists(), "dry-run must not build");
}

#[test]
fn update_outside_a_maw_rs_checkout_names_the_precondition() {
    let fixture = UpdateFixture::new("wrong-directory");
    fixture.write_cargo(0, true);
    fixture.write_flock();
    fixture.write_install();
    fixture.write_pm2(true);
    fs::create_dir_all(fixture.path("elsewhere")).expect("elsewhere");

    let output = Command::new(env!("CARGO_BIN_EXE_maw-rs"))
        .args(["update"])
        .current_dir(fixture.path("elsewhere"))
        .env_clear()
        .env("PATH", format!("{}:/usr/bin:/bin", fixture.path("bin").display()))
        .env("HOME", &fixture.root)
        .env("MARKER_LOG", fixture.path("marker.log"))
        .output()
        .expect("run update");

    assert_eq!(output.status.code(), Some(1));
    assert!(stderr_of(&output).contains("maw-rs checkout"), "{}", stderr_of(&output));
    assert!(fixture.markers().is_empty(), "the precondition is checked before anything runs");
}

#[test]
fn update_rejects_a_ref_argument_and_says_what_it_does_instead() {
    let fixture = UpdateFixture::new("no-ref");
    fixture.write_cargo(0, true);
    fixture.write_flock();
    fixture.write_install();
    fixture.write_pm2(true);

    let output = fixture.run(&["update", "alpha"]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = stderr_of(&output);
    assert!(stderr.contains("takes no argument \"alpha\""), "{stderr}");
    assert!(!stderr.contains("native-only in maw-rs"), "the maw-js tombstone is gone: {stderr}");
    assert!(fixture.markers().is_empty());
}
