//! Pure `maw bring` policy helpers ported from maw-js.
//!
//! This crate intentionally excludes tmux/runtime IO. Behavior is locked by
//! maw-js portable fixtures for `src/commands/shared/bring-flags.ts`.

/// Parsed `maw bring --to` destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BringToTarget {
    pub session: String,
    pub window: Option<String>,
}

/// Translate `--to <session[:window]>` to wake-shaped flags.
///
/// `--to` without a following value is preserved so downstream parsing can
/// surface the same error class as maw-js.
#[must_use]
pub fn translate_bring_to_flag(argv: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(argv.len());
    let mut index = 0;
    while index < argv.len() {
        let arg = &argv[index];
        if arg == "--to" && index + 1 < argv.len() {
            index += 1;
            let target = parse_bring_to_target(&argv[index]);
            out.push("--session".to_owned());
            out.push(target.session.clone());
            if let Some(window) = target.window {
                out.push("--split-target".to_owned());
                out.push(format!("{}:{window}", target.session));
            }
        } else {
            out.push(arg.clone());
        }
        index += 1;
    }
    out
}

/// Parse a `--to` value that may contain a destination window.
#[must_use]
pub fn parse_bring_to_target(value: &str) -> BringToTarget {
    let Some((session, window)) = value.split_once(':') else {
        return BringToTarget {
            session: value.to_owned(),
            window: None,
        };
    };
    BringToTarget {
        session: session.to_owned(),
        window: (!window.is_empty()).then(|| window.to_owned()),
    }
}

/// Detect whether a split target points at the caller's own pane/window.
#[must_use]
pub fn is_self_bring(target: &str, caller_session_window: Option<&str>) -> bool {
    let Some(caller_session_window) = caller_session_window else {
        return false;
    };
    if target.is_empty() {
        return false;
    }

    let target_no_pane = strip_numeric_pane_suffix(target);
    if target_no_pane == caller_session_window {
        return true;
    }

    let caller_session = caller_session_window
        .split_once(':')
        .map_or(caller_session_window, |(session, _)| session);
    !target_no_pane.contains(':') && target_no_pane == caller_session
}

fn strip_numeric_pane_suffix(value: &str) -> &str {
    let Some((head, suffix)) = value.rsplit_once('.') else {
        return value;
    };
    if !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit()) {
        head
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotted_window_names_are_not_pane_suffixes() {
        assert_eq!(strip_numeric_pane_suffix("s:oracle.v2"), "s:oracle.v2");
        assert_eq!(strip_numeric_pane_suffix("s:oracle.12"), "s:oracle");
    }
}
