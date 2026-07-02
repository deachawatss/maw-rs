use maw_cli::{run_cli, CliOutput};

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn ls_plan_defaults_to_compact_live_oracle_roster() {
    let output = run_cli(&args(&[
        "ls",
        "--plan-json",
        "--pane",
        "%1|claude|50-mawjs:1.0|mawjs|100|/repo|1700000000",
        "--pane",
        "%2|zsh|scratch:1.0|scratch|101|/tmp|1699999700",
    ]));

    assert_eq!(output.code, 0, "{}", output.stderr);
    assert_eq!(
        output.stdout,
        concat!(
            "{\"command\":\"ls\",\"mode\":\"compact\",\"scope\":\"local\",\"json\":true,",
            "\"sessions\":[{\"session\":\"50-mawjs\",\"status\":\"stale\",\"panes\":1,\"agents\":1}]}
"
        )
    );
}

#[test]
fn ls_plan_verbose_json_keeps_channels_filter_and_statuses() {
    let output = run_cli(&args(&[
        "ls",
        "--json",
        "--verbose",
        "--channels",
        "alpha",
        "--now",
        "1700000100",
        "--pane",
        "%1|claude|mawjs-oracle-discord:1.0|chan|100|/repo|1700000090",
        "--pane",
        "%2|node|alpha-worker:2.0|worker|101|/repo|1700000050",
    ]));

    assert_eq!(output.code, 0, "{}", output.stderr);
    assert_eq!(
        output.stdout,
        concat!(
            "{\"command\":\"ls\",\"mode\":\"verbose\",\"scope\":\"local\",\"json\":true,",
            "\"panes\":[{\"id\":\"%2\",\"target\":\"alpha-worker:2.0\",\"session\":\"alpha-worker\",\"command\":\"node\",\"title\":\"worker\",\"status\":\"idle\",\"ageSec\":50,\"agent\":true}]}
"
        )
    );
}

#[test]
fn ls_plan_active_recent_and_help_match_maw_js_surface() {
    let output = run_cli(&args(&[
        "ls",
        "--active",
        "1h",
        "--recent",
        "1",
        "--plan-json",
        "--now",
        "1700000000",
        "--session-created",
        "old-session=100",
        "--session-created",
        "new-session=300",
        "--pane",
        "%1|claude|old-session:1.0|old|100|/repo|1699999900",
        "--pane",
        "%2|claude|new-session:1.0|new|101|/repo|1699999950",
    ]));

    assert_eq!(output.code, 0, "{}", output.stderr);
    assert_eq!(
        output.stdout,
        concat!(
            "{\"command\":\"ls\",\"mode\":\"compact\",\"scope\":\"local\",\"json\":true,",
            "\"activeThresholdSec\":3600,\"recentLimit\":1,",
            "\"sessions\":[{\"session\":\"new-session\",\"status\":\"idle\",\"panes\":1,\"agents\":1,\"created\":300,\"lastActivityAgeSec\":50}]}
"
        )
    );

    let help = run_cli(&args(&["ls", "--help"]));
    assert_eq!(help.code, 0);
    assert!(help.stdout.contains("maw ls --active [30m]"));
    assert!(help.stdout.contains("maw ls <peer>"));
}

#[test]
fn ls_plan_rejects_unknown_flags() {
    let output = run_cli(&args(&["ls", "--bad"]));
    assert_eq!(
        output,
        CliOutput {
            code: 2,
            stdout: String::new(),
            stderr: "ls: unknown argument --bad\nusage: maw-rs ls [<filter>] [--all] [--json|--plan-json] [--compact|-c] [--verbose|-v] [--recent|-r [N]] [--active [30m|1h]] [--federation] [--fleet-only] [--node <node>] [--verify] [--fix] [--channels] [--pane <id|command|target|title|pid|cwd|last_activity>]...\n".to_owned(),
        }
    );
}
