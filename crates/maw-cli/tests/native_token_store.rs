use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

const SECRET_VALUE: &str = "super-secret-token-value";

fn epic55_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_maw-rs"))
}

fn epic55_temp_dir(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("maw-rs-epic55-token-{name}-{stamp}"));
    fs::create_dir_all(&path).expect("temp dir");
    path
}

fn epic55_seed_fake_pass(root: &Path) {
    let pass = root.join("fake/pass");
    fs::create_dir_all(pass.join("claude")).expect("claude pass");
    fs::create_dir_all(pass.join("envrc")).expect("envrc pass");
    fs::write(pass.join("claude/token-nova"), "nova-token-value\n").expect("nova token");
    fs::write(pass.join("claude/token-pym"), format!("{SECRET_VALUE}\n")).expect("pym token");
    fs::write(
        pass.join("envrc/demo"),
        "export CLAUDE_TOKEN_NAME=\"nova\"\nexport CLAUDE_CODE_OAUTH_TOKEN=\"$(pass show claude/token-nova)\"\n",
    )
    .expect("demo envrc");
}

fn epic55_run(root: &Path, cwd: &Path, args: &[&str]) -> Output {
    Command::new(epic55_bin())
        .args(args)
        .current_dir(cwd)
        .env("MAW_JS_REF_DIR", "/nonexistent")
        .env("MAW_RS_TOKEN_FAKE_ROOT", root.join("fake"))
        .env("HOME", root.join("home"))
        .env_remove("CLAUDE_TOKEN_NAME")
        .output()
        .expect("run maw-rs")
}

#[test]
fn epic55_token_list_matches_committed_golden_names_only() {
    let root = epic55_temp_dir("list");
    epic55_seed_fake_pass(&root);
    let cwd = root.join("repo");
    fs::create_dir_all(cwd.join(".git")).expect("cwd");
    fs::write(
        cwd.join(".envrc"),
        "export CLAUDE_TOKEN_NAME=\"nova\"\nexport CLAUDE_CODE_OAUTH_TOKEN=\"$(pass show claude/token-nova)\"\n",
    )
    .expect("envrc");

    let output = epic55_run(&root, &cwd, &["token", "list"]);

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout");
    assert_eq!(stdout, include_str!("fixtures/epic55/token-list.stdout"));
    assert!(!stdout.contains(SECRET_VALUE));
    assert_eq!(String::from_utf8(output.stderr).expect("stderr"), "");
}

#[test]
fn epic55_token_use_writes_reference_atomically_without_value() {
    let root = epic55_temp_dir("use");
    epic55_seed_fake_pass(&root);
    let cwd = root.join("repo");
    fs::create_dir_all(cwd.join(".git")).expect("cwd");
    fs::write(
        cwd.join(".envrc"),
        "export KEEP_ME=1\nexport CLAUDE_CODE_OAUTH_TOKEN=old-literal\nexport CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1\n",
    )
    .expect("old envrc");

    let output = epic55_run(&root, &cwd, &["token", "use", "pym"]);

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout"),
        include_str!("fixtures/epic55/token-use.stdout")
    );
    assert_eq!(String::from_utf8(output.stderr).expect("stderr"), "");
    let envrc = fs::read_to_string(cwd.join(".envrc")).expect("envrc");
    assert!(envrc.contains("export KEEP_ME=1"));
    assert!(envrc.contains("export CLAUDE_TOKEN_NAME=\"pym\""));
    assert!(envrc.contains("export CLAUDE_CODE_OAUTH_TOKEN=\"$(pass show claude/token-pym)\""));
    assert!(!envrc.contains(SECRET_VALUE));
    assert!(!envrc.contains("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS"));
}

#[test]
fn epic55_token_name_shorthand_switches_active_account() {
    let root = epic55_temp_dir("shorthand");
    epic55_seed_fake_pass(&root);
    let cwd = root.join("repo");
    fs::create_dir_all(cwd.join(".git")).expect("cwd");
    fs::write(cwd.join(".envrc"), "export KEEP_ME=1\n").expect("old envrc");

    let output = epic55_run(&root, &cwd, &["token", "pym"]);

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout"),
        include_str!("fixtures/epic55/token-use.stdout")
    );
    assert_eq!(String::from_utf8(output.stderr).expect("stderr"), "");
    let envrc = fs::read_to_string(cwd.join(".envrc")).expect("envrc");
    assert!(envrc.contains("export KEEP_ME=1"));
    assert!(envrc.contains("export CLAUDE_TOKEN_NAME=\"pym\""));
    assert!(envrc.contains("export CLAUDE_CODE_OAUTH_TOKEN=\"$(pass show claude/token-pym)\""));
    assert!(!envrc.contains(SECRET_VALUE));
    assert!(!envrc.contains("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS"));
}

#[test]
fn epic55_token_save_transits_envrc_via_pass_stdin_and_hides_pass_failures() {
    let root = epic55_temp_dir("save");
    epic55_seed_fake_pass(&root);
    let cwd = root.join("repo");
    fs::create_dir_all(cwd.join(".git")).expect("cwd");
    fs::write(
        cwd.join(".envrc"),
        "export CLAUDE_TOKEN_NAME=\"nova\"\nexport CLAUDE_CODE_OAUTH_TOKEN=\"$(pass show claude/token-nova)\"\n",
    )
    .expect("envrc");

    let saved = epic55_run(&root, &cwd, &["token", "save", "repo", "--force"]);
    assert!(
        saved.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&saved.stderr)
    );
    let stored = fs::read_to_string(root.join("fake/pass/envrc/repo")).expect("saved envrc");
    assert!(
        stored.contains("$(pass show claude/token-nova)"),
        "fake pass receives stdin payload"
    );

    let failed = Command::new(epic55_bin())
        .args(["token", "save", "repo", "--force"])
        .current_dir(&cwd)
        .env("MAW_JS_REF_DIR", "/nonexistent")
        .env("MAW_RS_TOKEN_FAKE_ROOT", root.join("fake"))
        .env("MAW_RS_TOKEN_FAKE_FAIL", "insert")
        .output()
        .expect("run failed save");
    assert!(!failed.status.success());
    let stderr = String::from_utf8(failed.stderr).expect("stderr");
    assert!(stderr.contains("pass insert failed"));
    assert!(!stderr.contains(SECRET_VALUE));
}

#[test]
fn epic55_token_load_requires_force_for_non_tty_overwrite_and_rejects_literal_secret_envrc() {
    let root = epic55_temp_dir("load");
    epic55_seed_fake_pass(&root);
    let cwd = root.join("repo");
    fs::create_dir_all(cwd.join(".git")).expect("cwd");
    fs::write(cwd.join(".envrc"), "export KEEP=1\n").expect("existing envrc");

    let skipped = epic55_run(&root, &cwd, &["token", "load", "demo"]);
    assert!(
        skipped.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&skipped.stderr)
    );
    assert_eq!(
        String::from_utf8(skipped.stdout).expect("stdout"),
        "Skipped (would overwrite .envrc; envrc/demo)\n"
    );
    assert_eq!(
        fs::read_to_string(cwd.join(".envrc")).expect("envrc"),
        "export KEEP=1\n"
    );

    let loaded = epic55_run(&root, &cwd, &["token", "load", "demo", "--force"]);
    assert!(
        loaded.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&loaded.stderr)
    );
    let envrc = fs::read_to_string(cwd.join(".envrc")).expect("envrc");
    assert!(envrc.contains("$(pass show claude/token-nova)"));
    assert!(!envrc.contains(SECRET_VALUE));

    fs::write(
        root.join("fake/pass/envrc/bad"),
        format!("export CLAUDE_CODE_OAUTH_TOKEN={SECRET_VALUE}\n"),
    )
    .expect("bad envrc");
    let rejected = epic55_run(&root, &cwd, &["token", "load", "bad", "--force"]);
    assert!(!rejected.status.success());
    let stderr = String::from_utf8(rejected.stderr).expect("stderr");
    assert!(stderr.contains("refusing to write"));
    assert!(!stderr.contains(SECRET_VALUE));
}

#[test]
fn epic55_token_scan_outputs_names_only_and_guards_bad_names_before_io() {
    let root = epic55_temp_dir("scan");
    epic55_seed_fake_pass(&root);
    let cwd = root.join("repo");
    let ghq_repo = root.join("ghq/github.com/acme/demo");
    fs::create_dir_all(cwd.join(".git")).expect("cwd");
    fs::create_dir_all(&ghq_repo).expect("ghq repo");
    fs::write(
        ghq_repo.join(".envrc"),
        format!("export CLAUDE_CODE_OAUTH_TOKEN={SECRET_VALUE}\n"),
    )
    .expect("repo envrc");

    let output = Command::new(epic55_bin())
        .args(["token", "scan"])
        .current_dir(&cwd)
        .env("MAW_JS_REF_DIR", "/nonexistent")
        .env("MAW_RS_TOKEN_FAKE_ROOT", root.join("fake"))
        .env("GHQ_ROOT", root.join("ghq"))
        .env("HOME", root.join("home"))
        .env_remove("CLAUDE_TOKEN_NAME")
        .output()
        .expect("scan");
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout");
    assert!(stdout.contains("unknown (1 repos)"), "{stdout}");
    assert!(stdout.contains("acme/demo *"), "{stdout}");
    assert!(!stdout.contains("pym"), "{stdout}");
    assert!(!stdout.contains(SECRET_VALUE));

    let guarded = epic55_run(&root, &cwd, &["token", "use", "../bad"]);
    assert!(!guarded.status.success());
    assert!(String::from_utf8(guarded.stderr)
        .expect("stderr")
        .contains("invalid argument value"));
}

#[test]
fn epic55_token_dispatch_is_final_native_slice() {
    assert_eq!(
        maw_cli::dispatcher_status("token"),
        maw_cli::DispatchKind::Native
    );
    assert_eq!(
        maw_cli::dispatcher_status("tokens"),
        maw_cli::DispatchKind::Native
    );
}

#[test]
fn token_use_refuses_the_home_directory() {
    let root = epic55_temp_dir("home-guard");
    epic55_seed_fake_pass(&root);
    let home = root.join("home");
    fs::create_dir_all(home.join(".git")).expect("home");

    let output = epic55_run(&root, &home, &["token", "use", "nova"]);

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("stderr");
    assert!(stderr.contains("home directory"), "{stderr}");
    assert!(stderr.contains(&home.display().to_string()), "{stderr}");
    assert!(!home.join(".envrc").exists());
}

#[test]
fn token_use_refuses_a_folder_outside_a_git_repo() {
    let root = epic55_temp_dir("repo-guard");
    epic55_seed_fake_pass(&root);
    let plain = root.join("plain");
    fs::create_dir_all(&plain).expect("plain");

    for args in [
        &["token", "use", "nova"][..],
        &["token", "nova"][..],
        &["token", "load", "demo", "--force"][..],
    ] {
        let output = epic55_run(&root, &plain, args);
        assert!(!output.status.success(), "{args:?}");
        let stderr = String::from_utf8(output.stderr).expect("stderr");
        assert!(
            stderr.contains("not inside a git repo"),
            "{args:?}: {stderr}"
        );
        assert!(!plain.join(".envrc").exists(), "{args:?}");
    }
}

#[test]
fn token_use_works_in_a_subfolder_of_a_git_repo() {
    let root = epic55_temp_dir("subdir");
    epic55_seed_fake_pass(&root);
    let repo = root.join("repo");
    let sub = repo.join("crates/app");
    fs::create_dir_all(repo.join(".git")).expect("repo");
    fs::create_dir_all(&sub).expect("sub");

    let output = epic55_run(&root, &sub, &["token", "use", "nova"]);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let envrc = fs::read_to_string(sub.join(".envrc")).expect("envrc");
    assert!(envrc.contains("pass show claude/token-nova"));
    assert!(!envrc.contains("nova-token-value"));
}

#[test]
fn token_use_refuses_a_subfolder_when_the_home_directory_is_the_repo() {
    let root = epic55_temp_dir("home-repo");
    epic55_seed_fake_pass(&root);
    let home = root.join("home");
    let downloads = home.join("Downloads");
    fs::create_dir_all(home.join(".git")).expect("home repo");
    fs::create_dir_all(&downloads).expect("downloads");

    let output = epic55_run(&root, &downloads, &["token", "use", "nova"]);

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("stderr");
    assert!(stderr.contains("not inside a git repo"), "{stderr}");
    assert!(!downloads.join(".envrc").exists());
}

#[test]
fn token_use_refuses_before_it_reads_pass() {
    let root = epic55_temp_dir("guard-first");
    epic55_seed_fake_pass(&root);
    let plain = root.join("plain");
    fs::create_dir_all(&plain).expect("plain");

    let output = epic55_run(&root, &plain, &["token", "use", "missing"]);

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("stderr");
    assert!(stderr.contains("not inside a git repo"), "{stderr}");
    assert!(!stderr.contains("not found in pass"), "{stderr}");
}

#[test]
fn token_current_reads_the_name_direnv_loaded_from_a_parent_folder() {
    let root = epic55_temp_dir("current-env");
    epic55_seed_fake_pass(&root);
    let sub = root.join("repo/crates/app");
    fs::create_dir_all(&sub).expect("sub");

    let output = Command::new(epic55_bin())
        .args(["token", "current"])
        .current_dir(&sub)
        .env("MAW_JS_REF_DIR", "/nonexistent")
        .env("MAW_RS_TOKEN_FAKE_ROOT", root.join("fake"))
        .env("HOME", root.join("home"))
        .env("CLAUDE_TOKEN_NAME", "gale")
        .output()
        .expect("run maw-rs");

    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).expect("stdout"), "gale\n");
}

#[test]
fn token_use_no_longer_takes_the_no_team_flag() {
    let root = epic55_temp_dir("no-team");
    epic55_seed_fake_pass(&root);
    let cwd = root.join("repo");
    fs::create_dir_all(cwd.join(".git")).expect("cwd");

    let output = epic55_run(&root, &cwd, &["token", "use", "pym", "--no-team"]);

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("stderr");
    assert!(stderr.contains("unknown flag --no-team"), "{stderr}");
    assert!(!cwd.join(".envrc").exists());
}

#[test]
fn token_load_refuses_any_literal_claude_secret() {
    let root = epic55_temp_dir("load-literal");
    epic55_seed_fake_pass(&root);
    let cwd = root.join("repo");
    fs::create_dir_all(cwd.join(".git")).expect("cwd");
    let cases = [
        ("other-name", "export ANTHROPIC_API_KEY=sk-ant-api03-secretvalue\n"),
        (
            "comment-bypass",
            "export CLAUDE_CODE_OAUTH_TOKEN=sk-ant-oat01-secretvalue # $(pass show claude/token-nova)\n",
        ),
        ("in-a-comment", "# sk-ant-oat01-secretvalue\nexport KEEP=1\n"),
    ];
    for (name, content) in cases {
        fs::write(root.join("fake/pass/envrc").join(name), content).expect("saved envrc");

        let output = epic55_run(&root, &cwd, &["token", "load", name, "--force"]);

        assert!(!output.status.success(), "{name}");
        let stderr = String::from_utf8(output.stderr).expect("stderr");
        assert!(stderr.contains("refusing to write"), "{name}: {stderr}");
        assert!(!stderr.contains("secretvalue"), "{name}: {stderr}");
        assert!(!cwd.join(".envrc").exists(), "{name}");
    }
}

#[test]
fn token_use_removes_an_agent_teams_line_without_export() {
    let root = epic55_temp_dir("bare-teams");
    epic55_seed_fake_pass(&root);
    let cwd = root.join("repo");
    fs::create_dir_all(cwd.join(".git")).expect("cwd");
    fs::write(
        cwd.join(".envrc"),
        "KEEP=1\nCLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1\n",
    )
    .expect("envrc");

    let output = epic55_run(&root, &cwd, &["token", "use", "pym"]);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let envrc = fs::read_to_string(cwd.join(".envrc")).expect("envrc");
    assert!(envrc.contains("KEEP=1"));
    assert!(!envrc.contains("AGENT_TEAMS"), "{envrc}");
}

#[test]
fn token_load_checks_the_assigned_value_not_the_whole_line() {
    let root = epic55_temp_dir("load-value");
    epic55_seed_fake_pass(&root);
    let cwd = root.join("repo");
    fs::create_dir_all(cwd.join(".git")).expect("cwd");
    fs::write(
        root.join("fake/pass/envrc/tail-ref"),
        "export CLAUDE_CODE_OAUTH_TOKEN=abc123literal # $(pass show claude/token-nova)\n",
    )
    .expect("tail-ref");
    fs::write(
        root.join("fake/pass/envrc/unset"),
        "unset CLAUDE_CODE_OAUTH_TOKEN\nexport CLAUDE_TOKEN_NAME=\"nova\"\nCLAUDE_CODE_OAUTH_TOKEN=\"$(pass show claude/token-nova)\"\n",
    )
    .expect("unset");

    let refused = epic55_run(&root, &cwd, &["token", "load", "tail-ref", "--force"]);
    assert!(!refused.status.success());
    let stderr = String::from_utf8(refused.stderr).expect("stderr");
    assert!(stderr.contains("refusing to write"), "{stderr}");
    assert!(!stderr.contains("abc123literal"), "{stderr}");

    let loaded = epic55_run(&root, &cwd, &["token", "load", "unset", "--force"]);
    assert!(
        loaded.status.success(),
        "{}",
        String::from_utf8_lossy(&loaded.stderr)
    );
}

#[test]
fn token_save_refuses_an_envrc_with_a_literal_secret() {
    let root = epic55_temp_dir("save-literal");
    epic55_seed_fake_pass(&root);
    let cwd = root.join("repo");
    fs::create_dir_all(cwd.join(".git")).expect("cwd");
    fs::write(
        cwd.join(".envrc"),
        "export ANTHROPIC_API_KEY=sk-ant-api03-secretvalue\n",
    )
    .expect("envrc");

    let output = epic55_run(&root, &cwd, &["token", "save", "leaky", "--force"]);

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("stderr");
    assert!(stderr.contains("refusing"), "{stderr}");
    assert!(!stderr.contains("secretvalue"), "{stderr}");
    assert!(!root.join("fake/pass/envrc/leaky").exists());
}

#[test]
fn token_use_checks_the_token_exists_without_reading_its_value() {
    let root = epic55_temp_dir("use-no-show");
    epic55_seed_fake_pass(&root);
    let cwd = root.join("repo");
    fs::create_dir_all(cwd.join(".git")).expect("cwd");

    let output = Command::new(epic55_bin())
        .args(["token", "use", "pym"])
        .current_dir(&cwd)
        .env("MAW_JS_REF_DIR", "/nonexistent")
        .env("MAW_RS_TOKEN_FAKE_ROOT", root.join("fake"))
        .env("MAW_RS_TOKEN_FAKE_FAIL", "show")
        .env("HOME", root.join("home"))
        .env_remove("CLAUDE_TOKEN_NAME")
        .output()
        .expect("run maw-rs");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let missing = epic55_run(&root, &cwd, &["token", "use", "nobody"]);
    assert!(!missing.status.success());
    assert!(String::from_utf8(missing.stderr)
        .expect("stderr")
        .contains("not found in pass"));
}

#[test]
fn token_use_gives_each_token_its_own_secure_storage_profile() {
    let root = epic55_temp_dir("profile");
    epic55_seed_fake_pass(&root);
    let cwd = root.join("repo");
    fs::create_dir_all(cwd.join(".git")).expect("cwd");
    fs::write(
        cwd.join(".envrc"),
        "export KEEP_ME=1\nexport CLAUDE_CODE_OAUTH_TOKEN=\"$(pass show claude/token-nova)\"\nexport CLAUDE_SECURESTORAGE_CONFIG_DIR=\"$HOME/.config/claude/profiles/nova\"\n",
    )
    .expect("old envrc");

    let output = epic55_run(&root, &cwd, &["token", "use", "pym"]);

    assert!(output.status.success(), "stderr={}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(
        fs::read_to_string(cwd.join(".envrc")).expect("envrc"),
        "export KEEP_ME=1\n\nexport CLAUDE_TOKEN_NAME=\"pym\"\nexport CLAUDE_CODE_OAUTH_TOKEN=\"$(pass show claude/token-pym)\"\nexport CLAUDE_SECURESTORAGE_CONFIG_DIR=\"$HOME/.config/claude/profiles/pym\"\n"
    );
}
