use super::*;
use std::{collections::BTreeMap, fs};
use twilight_model::gateway::{event::Event, Intents};

const MAP_FILE: &str = "ψ/memory/discord-channel-map.json";

struct Config {
    bot_id: String,
    wind_user_id: String,
    channels: BTreeMap<String, String>,
    map_path: std::path::PathBuf,
}

struct Inbound {
    channel_id: String,
    author_id: String,
    is_bot: bool,
    content: String,
}

pub(super) async fn serve(
    env: &DiscordEnv,
    rest: &dyn DiscordRest,
    args: &[String],
    relay: &dyn DiscordPaneRelay,
    log: &mut Vec<String>,
) -> bool {
    let Ok((oracle, dry_run, channel, message)) = parse_args(args) else {
        log.push("usage: maw discord serve <oracle> [--dry-run] [--channel <mapped-channel> --message <text>]".to_owned());
        return false;
    };
    let config = match load_config(env, &oracle) {
        Ok(config) => config,
        Err(error) => {
            log.push(format!("✗ discord serve: {error}"));
            return false;
        }
    };
    if dry_run {
        log.push(format!(
            "🛰 maw discord serve {oracle}: native gateway dry-run"
        ));
        log.push(format!("  channel map: {}", config.map_path.display()));
        log.push(format!("  mapped channels: {}", config.channels.len()));
        return true;
    }
    if let (Some(channel), Some(message)) = (channel, message) {
        return post_mapped_message(env, rest, &oracle, &config, &channel, &message, log).await;
    }
    run_gateway(env, &oracle, &config, relay, log).await
}

fn parse_args(args: &[String]) -> Result<(String, bool, Option<String>, Option<String>), ()> {
    let (mut oracle, mut dry_run, mut channel, mut message) = (None, false, None, None);
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--dry-run" => dry_run = true,
            "--channel" | "--message" => {
                let Some(value) = args.get(index + 1).filter(|value| !value.is_empty()) else {
                    return Err(());
                };
                if args[index] == "--channel" {
                    channel = Some(value.clone());
                } else {
                    message = Some(value.clone());
                }
                index += 1;
            }
            raw_oracle if raw_oracle.starts_with("--") => return Err(()),
            raw_oracle => {
                if oracle.is_some() {
                    return Err(());
                }
                let normalized = raw_oracle.strip_suffix("-oracle").unwrap_or(raw_oracle);
                if normalized.is_empty()
                    || !normalized
                        .chars()
                        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
                {
                    return Err(());
                }
                oracle = Some(normalized.to_owned());
            }
        }
        index += 1;
    }
    if channel.is_some() != message.is_some() {
        return Err(());
    }
    oracle
        .map(|oracle| (oracle, dry_run, channel, message))
        .ok_or(())
}

fn load_config(env: &DiscordEnv, oracle: &str) -> Result<Config, String> {
    let map_path = env
        .ghq_root
        .join("github.com/deachawatss")
        .join(format!("{oracle}-oracle"))
        .join(MAP_FILE);
    let value: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&map_path)
            .map_err(|_| format!("cannot read {}", map_path.display()))?,
    )
    .map_err(|_| "channel map is not valid JSON".to_owned())?;
    let id = |key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_str)
            .filter(|id| is_numeric_snowflake(id))
            .map(ToOwned::to_owned)
            .ok_or_else(|| format!("channel map has no valid {key}"))
    };
    let (bot_id, wind_user_id) = (id("bot_id")?, id("wind_user_id")?);
    let mut channels = BTreeMap::new();
    collect_channels(&value, &mut channels);
    if channels.is_empty() {
        return Err("channel map has no mapped channel IDs".to_owned());
    }
    Ok(Config {
        bot_id,
        wind_user_id,
        channels,
        map_path,
    })
}

fn collect_channels(value: &serde_json::Value, channels: &mut BTreeMap<String, String>) {
    let Some(object) = value.as_object() else {
        return;
    };
    for (name, value) in object {
        if matches!(name.as_str(), "server_id" | "bot_id" | "wind_user_id") {
            continue;
        }
        if let Some(id) = value.as_str().filter(|id| is_numeric_snowflake(id)) {
            channels.insert(name.clone(), id.to_owned());
        }
        collect_channels(value, channels);
    }
}

fn mapped_channel(config: &Config, channel: &str) -> Option<String> {
    config.channels.get(channel).cloned()
}

async fn post_mapped_message(
    env: &DiscordEnv,
    rest: &dyn DiscordRest,
    oracle: &str,
    config: &Config,
    channel: &str,
    message: &str,
    log: &mut Vec<String>,
) -> bool {
    let Some(channel_id) = mapped_channel(config, channel) else {
        log.push(format!("✗ channel '{channel}' not in channel map"));
        return false;
    };
    if message.trim().is_empty() || message.contains('\0') || message.chars().count() > 2_000 {
        log.push(
            "✗ discord serve message must be non-empty, NUL-free, and at most 2000 characters"
                .to_owned(),
        );
        return false;
    }
    let token = match gateway::resolve_gateway_token(
        env,
        &gateway::GatewayConfig::new(format!("{oracle}-oracle"), Intents::GUILD_MESSAGES),
    ) {
        Ok(token) => token.into_inner(),
        Err(error) => {
            log.push(format!("✗ discord serve: {error}"));
            return false;
        }
    };
    match rest
        .post_json(
            &format!("/channels/{channel_id}/messages"),
            &token,
            serde_json::json!({"content": message}),
        )
        .await
    {
        Ok(response) if (200..300).contains(&response.status) => {
            log.push(format!("  ✓ posted Discord message to {channel}"));
            true
        }
        Ok(response) => {
            log.push(format!("✗ Discord post failed: status {}", response.status));
            false
        }
        Err(_) => {
            log.push("✗ Discord post failed".to_owned());
            false
        }
    }
}

async fn run_gateway(
    env: &DiscordEnv,
    oracle: &str,
    config: &Config,
    relay: &dyn DiscordPaneRelay,
    log: &mut Vec<String>,
) -> bool {
    let handle = match gateway::spawn_gateway(
        env,
        gateway::GatewayConfig::new(
            format!("{oracle}-oracle"),
            Intents::GUILD_MESSAGES | Intents::MESSAGE_CONTENT,
        ),
    ) {
        Ok(handle) => handle,
        Err(error) => {
            log.push(format!("✗ discord serve: {error}"));
            return false;
        }
    };
    let mut events = handle.subscribe();
    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);
    eprintln!("maw discord serve {oracle}: native gateway running");
    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            event = events.recv() => if let Ok(event) = event {
                if let Some(inbound) = inbound(&event.event) {
                    if let Some((target, body)) = pane_message(config, oracle, &inbound) {
                        if let Err(error) = relay.relay(&target, &body).await { eprintln!("maw discord serve: {error}"); }
                    }
                }
            },
        }
    }
    let _ = handle.shutdown().await;
    log.push(format!("maw discord serve {oracle}: stopped"));
    true
}

fn inbound(event: &Event) -> Option<Inbound> {
    let Event::MessageCreate(message) = event else {
        return None;
    };
    Some(Inbound {
        channel_id: message.channel_id.get().to_string(),
        author_id: message.author.id.get().to_string(),
        is_bot: message.author.bot,
        content: message.content.clone(),
    })
}

fn pane_message(config: &Config, oracle: &str, inbound: &Inbound) -> Option<(String, String)> {
    if inbound.is_bot
        || inbound.author_id != config.wind_user_id
        || inbound.author_id == config.bot_id
    {
        return None;
    }
    let channel = config
        .channels
        .iter()
        .find_map(|(name, id)| (id == &inbound.channel_id).then_some(name))?;
    let content = clean_pane_content(&inbound.content)?;
    Some((oracle.to_owned(), format!("[discord:{channel}] {content}")))
}

fn clean_pane_content(content: &str) -> Option<String> {
    let mut clean = String::new();
    for character in content.chars() {
        if matches!(character, '\n' | '\r' | '\t') {
            clean.push(' ');
        } else if character.is_control() {
            return None;
        } else {
            clean.push(character);
        }
    }
    let clean = clean.trim();
    (!clean.is_empty()).then(|| clean.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoRest;

    impl DiscordRest for NoRest {
        fn get_json<'a>(
            &'a self,
            _path: &'a str,
            _token: &'a str,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<DiscordHttpResponse, String>> + Send + 'a>,
        > {
            Box::pin(async { Err("not called".to_owned()) })
        }
    }

    #[tokio::test]
    async fn command_dispatch_uses_the_fork_owned_native_gateway_hook() {
        let root = PathBuf::from(format!(
            "/tmp/maw-discord-wind-serve-{}",
            std::process::id()
        ));
        let map_dir = root
            .join("github.com/deachawatss/gale-oracle")
            .join(MAP_FILE)
            .parent()
            .expect("map parent")
            .to_owned();
        std::fs::create_dir_all(&map_dir).expect("map directory");
        std::fs::write(
            map_dir.join("discord-channel-map.json"),
            r#"{"bot_id":"42","wind_user_id":"7","oracle_family":{"gale-log":"100"}}"#,
        )
        .expect("map");
        let env = DiscordEnv {
            home: root.clone(),
            ghq_root: root,
            hostname: "test".to_owned(),
        };
        let out = run_discord_command_with(
            &[
                "serve".to_owned(),
                "gale".to_owned(),
                "--dry-run".to_owned(),
            ],
            &env,
            &NoRest,
        )
        .await;

        assert!(out.stdout.contains("native gateway"));
    }

    #[test]
    fn parse_args_accepts_dry_run_before_the_oracle() {
        let args = ["--dry-run", "gale-oracle"]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();

        assert_eq!(parse_args(&args), Ok(("gale".to_owned(), true, None, None)));
    }

    #[test]
    fn parse_args_rejects_unknown_flags_as_oracles() {
        let args = ["--host"]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();

        assert_eq!(parse_args(&args), Err(()));
    }

    #[test]
    fn only_the_configured_wind_user_in_a_mapped_channel_reaches_the_oracle_pane() {
        let config = Config {
            bot_id: "42".to_owned(),
            wind_user_id: "7".to_owned(),
            channels: BTreeMap::from([("gale-log".to_owned(), "100".to_owned())]),
            map_path: PathBuf::new(),
        };
        let inbound = Inbound {
            channel_id: "100".to_owned(),
            author_id: "7".to_owned(),
            is_bot: false,
            content: "hello\nGale".to_owned(),
        };
        assert_eq!(
            pane_message(&config, "gale", &inbound),
            Some((
                "gale".to_owned(),
                "[discord:gale-log] hello Gale".to_owned()
            ))
        );
    }

    #[test]
    fn outbound_channels_must_be_named_in_the_oracle_map() {
        let config = Config {
            bot_id: "42".to_owned(),
            wind_user_id: "7".to_owned(),
            channels: BTreeMap::from([("gale-log".to_owned(), "100".to_owned())]),
            map_path: PathBuf::new(),
        };

        assert_eq!(mapped_channel(&config, "999"), None);
    }

    #[test]
    fn pane_relay_rejects_each_untrusted_message_variant() {
        let config = Config {
            bot_id: "42".to_owned(),
            wind_user_id: "7".to_owned(),
            channels: BTreeMap::from([("gale-log".to_owned(), "100".to_owned())]),
            map_path: PathBuf::new(),
        };
        let rejected = [
            Inbound {
                channel_id: "100".to_owned(),
                author_id: "8".to_owned(),
                is_bot: false,
                content: "wrong author".to_owned(),
            },
            Inbound {
                channel_id: "100".to_owned(),
                author_id: "7".to_owned(),
                is_bot: true,
                content: "bot".to_owned(),
            },
            Inbound {
                channel_id: "200".to_owned(),
                author_id: "7".to_owned(),
                is_bot: false,
                content: "unmapped channel".to_owned(),
            },
        ];
        for inbound in rejected {
            assert_eq!(pane_message(&config, "gale", &inbound), None);
        }
        let bot_config = Config {
            bot_id: "7".to_owned(),
            wind_user_id: "7".to_owned(),
            channels: BTreeMap::from([("gale-log".to_owned(), "100".to_owned())]),
            map_path: PathBuf::new(),
        };
        let bot_author = Inbound {
            channel_id: "100".to_owned(),
            author_id: "7".to_owned(),
            is_bot: false,
            content: "bot id".to_owned(),
        };
        assert_eq!(pane_message(&bot_config, "gale", &bot_author), None);
    }
}
