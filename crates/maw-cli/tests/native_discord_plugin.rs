use maw_cli::{dispatcher_status, DispatchKind};
use maw_discord::{run_discord_command_with, DiscordEnv, DiscordHttpResponse, DiscordRest};
use serde_json::json;
use std::{
    collections::BTreeMap,
    fs,
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::{Mutex as StdMutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

struct MockRest {
    calls: StdMutex<Vec<String>>,
    responses: BTreeMap<String, DiscordHttpResponse>,
}

impl DiscordRest for MockRest {
    fn get_json<'a>(
        &'a self,
        path: &'a str,
        _token: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<DiscordHttpResponse, String>> + Send + 'a>> {
        Box::pin(async move {
            assert!(path.starts_with('/'));
            assert!(!path.contains("://"));
            self.calls.lock().expect("calls").push(path.to_owned());
            self.responses
                .get(path)
                .cloned()
                .ok_or_else(|| format!("missing mock response: {path}"))
        })
    }
}

fn env_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

fn temp_dir(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("maw-rs-native-discord-{name}-{stamp}"));
    fs::create_dir_all(&path).expect("temp dir");
    path
}

fn env(root: &std::path::Path) -> DiscordEnv {
    DiscordEnv {
        home: root.join("home"),
        ghq_root: root.join("ghq"),
        hostname: "fleetbox.example".to_owned(),
    }
}

fn seed_bot(root: &std::path::Path, bot: &str) -> DiscordEnv {
    let env = env(root);
    fs::create_dir_all(env.home.join(".password-store/discord")).expect("pass dir");
    fs::write(
        env.home
            .join(".password-store/discord")
            .join(format!("{bot}-token.gpg")),
        "encrypted-placeholder\n",
    )
    .expect("token placeholder");
    let repo = env.ghq_root.join("github.com/Soul-Brews-Studio").join(bot);
    fs::create_dir_all(repo.join(".discord")).expect("discord dir");
    fs::write(
        repo.join(".discord/channel-map.json"),
        "{\n  \"general\": \"111\"\n}\n",
    )
    .expect("channel map");
    fs::write(
        repo.join(".discord/access.json"),
        "{\n  \"dmPolicy\": \"allowlist\",\n  \"allowFrom\": [],\n  \"groups\": {\n    \"111\": { \"requireMention\": true, \"allowFrom\": [\"42\"] }\n  },\n  \"pending\": {}\n}\n",
    )
    .expect("access");
    let oracle = env
        .ghq_root
        .join("github.com/Soul-Brews-Studio/discord-oracle/src");
    fs::create_dir_all(&oracle).expect("oracle");
    fs::write(
        oracle.join("state-dirs.ts"),
        format!(
            "export const STATE_DIRS = {{\n  \"{bot}\": \"/state/{bot}\",\n}};\nexport const ANCHORS = {{\n  \"{bot}\": \"fleetbox\",\n}};\n"
        ),
    )
    .expect("state dirs");
    env
}

#[tokio::test]
async fn discord_members_rejects_non_numeric_allow_from_without_rest_call() {
    let _guard = env_lock().lock().await;
    std::env::set_var("MAW_JS_REF_DIR", "/nonexistent");
    let root = temp_dir("invalid-user-id");
    let env = seed_bot(&root, "nova-oracle");
    std::env::set_var("DISCORD_BOT_TOKEN", "mock-token-never-logged");
    let access_path = env
        .ghq_root
        .join("github.com/Soul-Brews-Studio/nova-oracle/.discord/access.json");
    fs::write(
        access_path,
        "{\n  \"dmPolicy\": \"allowlist\",\n  \"allowFrom\": [],\n  \"groups\": {\n    \"111\": { \"requireMention\": true, \"allowFrom\": [\"@me/guilds\"] }\n  },\n  \"pending\": {}\n}\n",
    )
    .expect("access with invalid id");
    let rest = MockRest {
        calls: StdMutex::new(Vec::new()),
        responses: BTreeMap::new(),
    };

    let output = run_discord_command_with(
        &[
            "members".to_owned(),
            "nova-oracle".to_owned(),
            "general".to_owned(),
            "--json".to_owned(),
        ],
        &env,
        &rest,
    )
    .await;

    assert_eq!(output.code, 0);
    assert!(
        output.stdout.contains("\"invalid\": true"),
        "{}",
        output.stdout
    );
    assert!(output.stdout.contains("@me/guilds"), "{}", output.stdout);
    assert!(
        rest.calls.lock().expect("calls").is_empty(),
        "invalid allow_from id must not be interpolated into a Discord REST path"
    );
    std::env::remove_var("DISCORD_BOT_TOKEN");
}

#[test]
fn discord_is_native_not_bun_fallback() {
    assert_eq!(dispatcher_status("discord"), DispatchKind::Native);
}

#[tokio::test]
async fn discord_version_committed_golden_without_ref_checkout() {
    let _guard = env_lock().lock().await;
    std::env::set_var("MAW_JS_REF_DIR", "/nonexistent");
    let root = temp_dir("version");
    let env = env(&root);
    let rest = MockRest {
        calls: StdMutex::new(Vec::new()),
        responses: BTreeMap::new(),
    };

    let output = run_discord_command_with(&["version".to_owned()], &env, &rest).await;

    assert_eq!(output.code, 0);
    assert_eq!(
        output.stdout,
        include_str!("fixtures/native-discord/version.stdout")
    );
    assert_eq!(output.stderr, "");
}

#[tokio::test]
async fn discord_access_list_and_inventory_use_mocked_rest_only() {
    let _guard = env_lock().lock().await;
    std::env::set_var("MAW_JS_REF_DIR", "/nonexistent");
    let root = temp_dir("inventory");
    let env = seed_bot(&root, "nova-oracle");
    std::env::set_var("DISCORD_BOT_TOKEN", "mock-token-never-logged");
    let mut responses = BTreeMap::new();
    responses.insert(
        "/users/@me/guilds".to_owned(),
        DiscordHttpResponse {
            status: 200,
            body: json!([{ "id": "999", "name": "Guild One" }]),
            retry_after: None,
        },
    );
    responses.insert(
        "/guilds/999/channels".to_owned(),
        DiscordHttpResponse {
            status: 200,
            body: json!([{ "id": "111", "name": "general", "type": 0, "guild_id": "999" }]),
            retry_after: None,
        },
    );
    responses.insert(
        "/users/42".to_owned(),
        DiscordHttpResponse {
            status: 200,
            body: json!({ "id": "42", "username": "tk" }),
            retry_after: None,
        },
    );
    let rest = MockRest {
        calls: StdMutex::new(Vec::new()),
        responses,
    };

    let listed = run_discord_command_with(
        &[
            "access".to_owned(),
            "nova-oracle".to_owned(),
            "list".to_owned(),
        ],
        &env,
        &rest,
    )
    .await;
    assert!(listed
        .stdout
        .contains("🪪 maw discord access nova-oracle list"));
    assert!(listed.stdout.contains("general"));

    let inventory = run_discord_command_with(
        &["inventory".to_owned(), "nova-oracle".to_owned()],
        &env,
        &rest,
    )
    .await;
    assert!(
        inventory.stdout.contains("📋 nova-oracle — full inventory"),
        "{}",
        inventory.stdout
    );
    assert!(inventory.stdout.contains("✓ #general"));
    let calls = rest.calls.lock().expect("calls");
    assert!(calls.iter().all(|path| path.starts_with('/')));
    assert!(calls.contains(&"/users/@me/guilds".to_owned()));
    assert!(calls.contains(&"/guilds/999/channels".to_owned()));
    std::env::remove_var("DISCORD_BOT_TOKEN");
}
