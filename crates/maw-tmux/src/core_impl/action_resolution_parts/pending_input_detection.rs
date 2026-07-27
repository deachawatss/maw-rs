fn clamp_pty(value: u32, max: u32) -> u32 {
    value.clamp(1, max)
}

/// Strip common terminal control sequences that tmux captures from pane output.
#[must_use]
pub fn strip_tmux_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == 0x1b && bytes.get(index + 1) == Some(&b']') {
            index += 2;
            while index < bytes.len() {
                if bytes[index] == 0x07 {
                    index += 1;
                    break;
                }
                if bytes[index] == 0x1b && bytes.get(index + 1) == Some(&b'\\') {
                    index += 2;
                    break;
                }
                index += 1;
            }
            continue;
        }
        if bytes[index] == 0x1b && bytes.get(index + 1) == Some(&b'[') {
            index += 2;
            while index < bytes.len() && (bytes[index].is_ascii_digit() || bytes[index] == b';') {
                index += 1;
            }
            if index < bytes.len()
                && (bytes[index] == b'm' || bytes[index].is_ascii_uppercase())
            {
                index += 1;
                continue;
            }
            out.push('\u{1b}');
            out.push('[');
            continue;
        }
        let ch = input[index..].chars().next().unwrap_or_default();
        out.push(ch);
        index += ch.len_utf8();
    }
    out
}

/// Return true when captured pane output appears to have pending prompt input.
#[must_use]
pub fn pane_input_pending_from_capture(content: &str) -> bool {
    if codex_pasted_content_pending(content) {
        return true;
    }
    pane_pending_input_from_capture(content).is_some()
}

fn codex_pasted_content_pending(content: &str) -> bool {
    let lines = content
        .lines()
        .map(normalize_capture_line)
        .collect::<Vec<_>>();
    lines.iter().enumerate().rev().any(|(index, line)| {
        trailing_after_prompt_is_chrome(&lines, index)
            && matches!(
                prompt_line(line),
                PromptLine::Pending(input)
                    if input.contains("[Pasted Content") && input.contains("chars]")
            )
    })
}

/// Return the current prompt input from captured pane output when it appears pending.
#[must_use]
pub fn pane_pending_input_from_capture(content: &str) -> Option<String> {
    let lines = content
        .lines()
        .map(normalize_capture_line)
        .collect::<Vec<_>>();
    for (index, line) in lines.iter().enumerate().rev() {
        match prompt_line(line) {
            PromptLine::Pending(input) if trailing_after_prompt_is_chrome(&lines, index) => {
                return Some(input);
            }
            PromptLine::Empty if trailing_after_prompt_is_chrome(&lines, index) => return None,
            PromptLine::Pending(_) | PromptLine::Empty | PromptLine::None => {}
        }
    }
    None
}

/// Return true when captured pane output ends at an empty shell prompt.
///
/// This deliberately differs from [`pane_pending_input_from_capture`]: an empty
/// prompt is a positive readiness signal, while a capture without any recognized
/// prompt is unknown and must not be treated as a successful submission.
#[must_use]
pub fn pane_has_empty_prompt_from_capture(content: &str) -> bool {
    let lines = content
        .lines()
        .map(normalize_capture_line)
        .collect::<Vec<_>>();
    for (index, line) in lines.iter().enumerate().rev() {
        match prompt_line(line) {
            PromptLine::Empty if trailing_after_prompt_is_chrome(&lines, index) => return true,
            PromptLine::Pending(_) if trailing_after_prompt_is_chrome(&lines, index) => return false,
            PromptLine::Pending(_) | PromptLine::Empty | PromptLine::None => {}
        }
    }
    false
}

/// Return true when the last rendered line ends at a trailing-marker shell prompt.
///
/// [`pane_has_empty_prompt_from_capture`] only recognizes prompts whose marker is the
/// FIRST character of the line (`❯ `, `% `) — the shape starship and most zsh themes
/// render. The default Debian/Ubuntu bash `PS1` puts the marker LAST
/// (`user@host:~/path$ `), so that scan finds no prompt at all — forever — and a
/// caller polling for readiness times out on every stock Linux box.
///
/// Kept deliberately narrow: this only reports that the last non-blank line ENDS in a
/// marker. Anything typed after the marker (`user@host:~$ ls`) ends in that text
/// instead, so pending input still reads as not-ready. Callers should pair this with a
/// process-level check (`pane_current_command` is a shell) so readiness stays positive
/// evidence rather than a guess about arbitrary pane text.
#[must_use]
pub fn pane_has_trailing_marker_prompt_from_capture(content: &str) -> bool {
    let Some(last) = content
        .lines()
        .map(normalize_capture_line)
        .rev()
        .find(|line| !line.trim().is_empty())
    else {
        return false;
    };
    last.trim_end().chars().next_back().is_some_and(is_prompt_marker)
}

/// Classification of pane pending input against the exact text maw just sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingInputState {
    Cleared,
    MatchesSent,
    DifferentInput,
}

/// Classify captured pending input for duplicate-safe Enter retries.
#[must_use]
pub fn pending_input_state_from_capture(content: &str, sent: &str) -> PendingInputState {
    let Some(pending) = pane_pending_input_from_capture(content) else {
        return PendingInputState::Cleared;
    };
    if pending_input_matches_sent(&pending, sent) {
        PendingInputState::MatchesSent
    } else {
        PendingInputState::DifferentInput
    }
}

/// Return true only when the captured pending input is the text maw attempted to submit.
#[must_use]
pub fn pending_input_matches_sent(pending: &str, sent: &str) -> bool {
    let pending = normalize_pending_text(pending);
    let sent = normalize_pending_text(sent);
    if pending.is_empty() || sent.is_empty() {
        return false;
    }
    if pending == sent {
        return true;
    }
    sent.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .is_some_and(|first_line| pending == first_line)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PromptLine {
    Pending(String),
    Empty,
    None,
}

fn prompt_line(line: &str) -> PromptLine {
    let trimmed = line.trim_start();
    let mut chars = trimmed.chars();
    let Some(marker) = chars.next() else {
        return PromptLine::None;
    };
    if !is_prompt_marker(marker) {
        return PromptLine::None;
    }
    let rest = chars.as_str();
    if rest.is_empty() {
        return PromptLine::Empty;
    }
    let input = rest.trim_start();
    if input.len() == rest.len() {
        return PromptLine::None;
    }
    let input = input.trim_end();
    if input.is_empty() {
        PromptLine::Empty
    } else if marker == '›' && input == "Use /skills to list available skills" {
        PromptLine::Empty
    } else {
        PromptLine::Pending(input.to_owned())
    }
}

fn is_prompt_marker(ch: char) -> bool {
    matches!(ch, '#' | '$' | '%' | '>' | '›' | '❯' | '»')
}

fn trailing_after_prompt_is_chrome(lines: &[String], index: usize) -> bool {
    lines
        .iter()
        .skip(index + 1)
        .all(|line| line_is_tui_chrome(line))
}

fn line_is_tui_chrome(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return true;
    }
    if matches!(prompt_line(trimmed), PromptLine::Empty) {
        return true;
    }
    if trimmed
        .chars()
        .all(|ch| ch.is_whitespace() || matches!(ch, '─' | '━' | '╌' | '┄' | '┈' | '-' | '—'))
    {
        return true;
    }
    if matches!(
        trimmed.chars().next(),
        Some('🖥' | '📡' | '🟢' | '🟡' | '🔴' | '⏵' | '◯' | '⧉')
    ) {
        return true;
    }
    let lower = trimmed.to_lowercase();
    lower.starts_with("gpt-")
        || lower.starts_with("claude")
        || lower.starts_with("opus")
        || lower.starts_with("sonnet")
        || lower.starts_with("haiku")
        || lower.starts_with("fable")
        || lower.starts_with("? for shortcuts")
        || lower.contains("context left")
}

fn normalize_capture_line(line: &str) -> String {
    strip_tmux_ansi(line)
        .replace('\r', "")
        .replace('\u{a0}', " ")
        .replace('\u{200b}', "")
}

fn normalize_pending_text(text: &str) -> String {
    normalize_capture_line(text).trim().to_owned()
}
