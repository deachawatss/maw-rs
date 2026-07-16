use serde::Deserialize;
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_maw-rs"))
}

struct BgHarness {
    root: PathBuf,
}

impl BgHarness {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = PathBuf::from("/tmp").join(format!("mbg-{}-{stamp:x}", std::process::id()));
        fs::create_dir_all(root.join("tmux")).expect("tmux root");
        fs::create_dir_all(root.join("state")).expect("state root");
        Self { root }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(bin())
            .arg("bg")
            .args(args)
            .env_remove("TMUX")
            .env("TMUX_TMPDIR", self.root.join("tmux"))
            .env("XDG_STATE_HOME", self.root.join("state"))
            .env("MAW_XDG", "1")
            .output()
            .expect("run maw bg")
    }

    fn list(&self) -> Vec<BgJob> {
        let output = self.run(&["ls", "--json"]);
        assert!(
            output.status.success(),
            "stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).expect("bg ls JSON")
    }

    fn wait_for(&self, slug: &str, status: &str) -> BgJob {
        for _ in 0..30 {
            if let Some(job) = self
                .list()
                .into_iter()
                .find(|job| job.slug == slug && job.status == status)
            {
                return job;
            }
            thread::sleep(Duration::from_millis(100));
        }
        panic!("{slug} did not reach {status}: {:?}", self.list());
    }
}

impl Drop for BgHarness {
    fn drop(&mut self) {
        let _ = self.run(&["kill", "--all"]);
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BgJob {
    slug: String,
    status: String,
    exit_code: Option<u8>,
}

#[test]
fn bg_lists_success_failure_running_and_killed_terminal_states() {
    let harness = BgHarness::new();

    let success = harness.run(&["exit", "0", "--name", "issue83-success"]);
    assert!(
        success.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&success.stderr)
    );
    assert_eq!(
        harness.wait_for("issue83-success", "done").exit_code,
        Some(0)
    );

    let failure = harness.run(&["exit", "7", "--name", "issue83-failure"]);
    assert!(
        failure.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&failure.stderr)
    );
    assert_eq!(
        harness.wait_for("issue83-failure", "failed").exit_code,
        Some(7)
    );
    let text = harness.run(&["ls"]);
    assert!(
        text.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&text.stderr)
    );
    let text = String::from_utf8(text.stdout).expect("bg ls text");
    assert!(text.contains("done (exit 0)"), "{text}");
    assert!(text.contains("failed (exit 7)"), "{text}");

    let running = harness.run(&["sleep", "90", "--name", "issue83-running"]);
    assert!(
        running.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&running.stderr)
    );
    assert_eq!(
        harness.wait_for("issue83-running", "running").exit_code,
        None
    );

    let killed = harness.run(&["kill", "issue83-running"]);
    assert!(
        killed.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&killed.stderr)
    );
    assert_eq!(
        harness.wait_for("issue83-running", "killed").exit_code,
        None
    );
}
