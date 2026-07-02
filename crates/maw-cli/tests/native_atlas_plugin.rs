use std::{
    process::Command,
    sync::{Mutex, OnceLock},
};

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn fake_discord() -> &'static str {
    r#"{
  "bot": "nova-oracle",
  "gateway_events": ["heartbeat", "heartbeat-ack"],
  "guilds": [
    {
      "id": "123456789012345678",
      "name": "Fleet Lab",
      "channels": [
        { "id": "222222222222222222", "name": "ops", "type": 0, "enabled": true, "requireMention": true, "allowFrom": ["111111111111111111"] },
        { "id": "333333333333333333", "name": "general", "type": 0, "enabled": false, "requireMention": true, "allowFrom": [] },
        { "id": "444444444444444444", "name": "thread", "type": 11, "enabled": true, "requireMention": false, "allowFrom": [] }
      ]
    }
  ]
}"#
}

#[test]
fn atlas_default_committed_golden_without_ref_checkout() {
    let _guard = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let output = Command::new(env!("CARGO_BIN_EXE_maw-rs"))
        .args(["atlas", "nova-oracle"])
        .env("MAW_JS_REF_DIR", "/nonexistent")
        .env("MAW_RS_ATLAS_FAKE_DISCORD", fake_discord())
        .env("DISCORD_BOT_TOKEN", "mock-token-never-printed")
        .output()
        .expect("run atlas");
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout"),
        include_str!("fixtures/native-atlas/atlas-default.stdout")
    );
    let stderr = String::from_utf8(output.stderr).expect("stderr");
    assert!(!stderr.contains("mock-token-never-printed"), "{stderr}");
}

#[test]
fn atlas_json_redacts_token_and_validates_guild_id() {
    let _guard = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let output = Command::new(env!("CARGO_BIN_EXE_maw-rs"))
        .args([
            "atlas",
            "nova-oracle",
            "--guild",
            "123456789012345678",
            "--json",
        ])
        .env("MAW_JS_REF_DIR", "/nonexistent")
        .env("MAW_RS_ATLAS_FAKE_DISCORD", fake_discord())
        .env("DISCORD_BOT_TOKEN", "mock-token-never-printed")
        .output()
        .expect("run atlas json");
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout");
    assert!(stdout.contains("\"gatewayEvents\": 2"), "{stdout}");
    assert!(!stdout.contains("mock-token-never-printed"), "{stdout}");

    let rejected = Command::new(env!("CARGO_BIN_EXE_maw-rs"))
        .args(["atlas", "nova-oracle", "--guild", "abc"])
        .env("MAW_JS_REF_DIR", "/nonexistent")
        .env("MAW_RS_ATLAS_FAKE_DISCORD", fake_discord())
        .env("DISCORD_BOT_TOKEN", "mock-token-never-printed")
        .output()
        .expect("run atlas bad guild");
    assert!(!rejected.status.success());
    let stderr = String::from_utf8(rejected.stderr).expect("stderr");
    assert!(stderr.contains("invalid guild id"), "{stderr}");
    assert!(!stderr.contains("mock-token-never-printed"), "{stderr}");
}
