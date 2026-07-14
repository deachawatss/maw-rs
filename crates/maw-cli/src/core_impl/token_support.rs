// Token helpers shared by squad and buddy-workspace flows.
//
// The top-level `maw token` command remains intentionally absent: it mutates
// Anthropic authentication state and was removed by the fork security patch.

const TOKEN_SUPPORT_PASS_PREFIX: &str = "claude/token-";

fn token_detect_active_token(content: &str) -> Option<String> {
    let active = content
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    if let Some(value) = token_extract_between(&active, "CLAUDE_TOKEN_NAME=\"", "\"") {
        return Some(value);
    }
    if let Some(index) = active.find("pass show claude/token-") {
        let tail = &active[index + "pass show claude/token-".len()..];
        let name = tail
            .chars()
            .take_while(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
            })
            .collect::<String>();
        if !name.is_empty() {
            return Some(name);
        }
    }
    if let Some(variable) = token_extract_after(&active, "export CLAUDE_CODE_OAUTH_TOKEN=$") {
        let needle = format!("{variable}=\"$(pass show claude/token-");
        if let Some(value) = token_extract_between(&active, &needle, ")\"") {
            return Some(value);
        }
    }
    None
}

fn token_build_envrc_content(existing: &str, name: &str, no_team: bool) -> String {
    let mut token_lines = vec![
        format!("export CLAUDE_TOKEN_NAME=\"{name}\""),
        format!(
            "export CLAUDE_CODE_OAUTH_TOKEN=\"$(pass show {TOKEN_SUPPORT_PASS_PREFIX}{name})\""
        ),
    ];
    if !no_team {
        token_lines.push("export CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1".to_owned());
    }
    if existing.is_empty() {
        return format!("{}\n", token_lines.join("\n"));
    }

    let mut kept = Vec::new();
    for line in existing.split('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with("export CLAUDE_TOKEN_NAME=")
            || trimmed.starts_with("CLAUDE_TOKEN_NAME=")
            || trimmed.starts_with("export CLAUDE_CODE_OAUTH_TOKEN=")
            || trimmed.starts_with("CLAUDE_CODE_OAUTH_TOKEN=")
            || trimmed.starts_with("export CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=")
            || token_is_legacy_token_line(trimmed)
        {
            continue;
        }
        kept.push(line.to_owned());
    }
    while kept.last().is_some_and(|line| line.trim().is_empty()) {
        kept.pop();
    }

    let mut content = kept.join("\n");
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push('\n');
    content.push_str(&token_lines.join("\n"));
    content.push('\n');
    content
}

fn token_is_legacy_token_line(line: &str) -> bool {
    let rest = line.strip_prefix("export ").unwrap_or(line);
    ["TOKEN_PYM=", "TOKEN_DO=", "TOKEN_TING_TING="]
        .iter()
        .any(|prefix| rest.starts_with(prefix))
}

fn token_validate_name(kind: &str, value: &str) -> Result<(), String> {
    token_validate_cli_value(kind, value)?;
    if value.contains('/') || value.contains('\\') || value.contains(std::path::MAIN_SEPARATOR) {
        return Err(format!("maw token: invalid {kind}"));
    }
    if !value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.'))
    {
        return Err(format!("maw token: invalid {kind}"));
    }
    Ok(())
}

fn token_validate_cli_value(kind: &str, value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.starts_with('-')
        || value.contains('\0')
        || value.chars().any(char::is_control)
        || value == ".."
        || value.contains("../")
        || value.contains("..\\")
    {
        return Err(format!("maw token: invalid {kind} value"));
    }
    Ok(())
}

fn token_extract_between(content: &str, prefix: &str, suffix: &str) -> Option<String> {
    let start = content.find(prefix)? + prefix.len();
    let end = content[start..].find(suffix)? + start;
    Some(content[start..end].to_owned())
}

fn token_extract_after(content: &str, prefix: &str) -> Option<String> {
    let start = content.find(prefix)? + prefix.len();
    let name = content[start..]
        .chars()
        .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
        .collect::<String>();
    (!name.is_empty()).then_some(name)
}

#[cfg(test)]
mod token_support_tests {
    use super::{token_build_envrc_content, token_detect_active_token, token_validate_name};

    #[test]
    fn shared_helpers_keep_pass_indirection_and_reject_path_like_token_names() {
        let existing = "export OLD=keep\nexport CLAUDE_TOKEN_NAME=old\n";

        assert_eq!(
            token_build_envrc_content(existing, "blue", true),
            "export OLD=keep\n\nexport CLAUDE_TOKEN_NAME=\"blue\"\nexport CLAUDE_CODE_OAUTH_TOKEN=\"$(pass show claude/token-blue)\"\n"
        );
        assert_eq!(
            token_detect_active_token("export CLAUDE_CODE_OAUTH_TOKEN=\"$(pass show claude/token-blue)\""),
            Some("blue".to_owned())
        );
        assert!(token_validate_name("token name", "../escape").is_err());
    }
}
