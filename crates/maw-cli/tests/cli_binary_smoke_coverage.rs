use std::process::Command;

#[test]
fn maw_rs_binary_smoke_runs_main_and_prints_help() {
    let output = Command::new(env!("CARGO_BIN_EXE_maw-rs"))
        .arg("--help")
        .output()
        .expect("run maw-rs binary");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("stdout utf8");
    assert!(
        stdout.contains("usage: maw-rs <command> [args]"),
        "{stdout}"
    );
    assert!(output.stderr.is_empty());
}
