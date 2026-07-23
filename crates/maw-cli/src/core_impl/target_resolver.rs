fn resolve_local_tmux_runner_target<R: maw_tmux::TmuxRunner>(
    runner: &mut R,
    query: &str,
    command: &str,
) -> Result<String, String> {
    if query.starts_with('%') {
        return Ok(query.to_owned());
    }
    let sessions = route_sessions_from_tmux_runner(runner, command)?;
    resolve_local_tmux_target_from_sessions(query, &sessions)
}

fn route_sessions_from_tmux_runner<R: maw_tmux::TmuxRunner>(
    runner: &mut R,
    command: &str,
) -> Result<Vec<RouteSession>, String> {
    let raw = runner
        .run(
            "list-windows",
            &[
                "-a".to_owned(),
                "-F".to_owned(),
                "#{session_name}|||#{window_index}|||#{window_name}|||#{window_active}|||#{pane_current_path}".to_owned(),
            ],
        )
        .map_err(|error| format!("{command} target resolution failed: {}", error.message))?;
    Ok(tmux_sessions_to_route_sessions(maw_tmux::parse_list_all_windows(&raw)))
}

fn tmux_sessions_to_route_sessions(sessions: Vec<TmuxSession>) -> Vec<RouteSession> {
    sessions
        .into_iter()
        .map(tmux_session_to_route_session)
        .collect()
}

fn tmux_session_to_route_session(session: TmuxSession) -> RouteSession {
    RouteSession {
        name: session.name,
        source: None,
        windows: session
            .windows
            .into_iter()
            .map(|window| RouteWindow {
                index: window.index,
                name: window.name,
                active: window.active,
                kind: None,
            })
            .collect(),
    }
}

fn resolve_local_tmux_target_from_sessions(
    query: &str,
    sessions: &[RouteSession],
) -> Result<String, String> {
    match resolve_route_target(query, &RouteConfig::default(), sessions) {
        RouteResult::Local { target } | RouteResult::SelfNode { target } => Ok(target),
        RouteResult::Peer { node, target, .. } => Err(format!(
            "cross-node target '{query}' (node '{node}', target '{target}') is not supported"
        )),
        RouteResult::Error { detail, hint, .. } => {
            if let Some(hint) = hint {
                Err(format!("{detail} — {hint}"))
            } else {
                Err(detail)
            }
        }
    }
}

const ROUTE_AGENT_PANE_FORMAT: &str =
    "#{pane_id}|||#{pane_current_command}|||#{session_name}:#{window_index}.#{pane_index}|||#{pane_title}|||#{pane_pid}|||#{pane_current_path}|||#{window_activity}";

#[derive(Debug, Clone, PartialEq, Eq)]
struct RoutePaneError {
    reason: String,
    detail: String,
    hint: Option<String>,
}

fn route_result_refuse_ambiguous_agent_panes<R: maw_tmux::TmuxRunner>(
    query: &str,
    result: RouteResult,
    runner: &mut R,
) -> RouteResult {
    if is_self_target_alias(query) {
        return result;
    }
    match result {
        RouteResult::Local { target } => {
            match resolve_window_agent_pane_target_with_runner(&target, runner) {
                Ok(target) => RouteResult::Local { target },
                Err(error) => route_pane_error(error),
            }
        }
        RouteResult::SelfNode { target } => {
            match resolve_window_agent_pane_target_with_runner(&target, runner) {
                Ok(target) => RouteResult::SelfNode { target },
                Err(error) => route_pane_error(error),
            }
        }
        other => other,
    }
}

fn resolve_window_agent_pane_target_with_runner<R: maw_tmux::TmuxRunner>(
    target: &str,
    runner: &mut R,
) -> Result<String, RoutePaneError> {
    if route_window_target_without_pane(target).is_none() {
        return Ok(target.to_owned());
    }
    let raw = runner
        .run(
            "list-panes",
            &[
                "-a".to_owned(),
                "-F".to_owned(),
                ROUTE_AGENT_PANE_FORMAT.to_owned(),
            ],
        )
        .map_err(|error| RoutePaneError {
            reason: "pane_inventory_unavailable".to_owned(),
            detail: format!(
                "could not enumerate panes for target '{target}'; refusing to guess a pane: {}",
                error.message
            ),
            hint: None,
        })?;
    resolve_window_agent_pane_target(target, &maw_tmux::parse_list_panes(&raw))
}

fn resolve_window_agent_pane_target(
    target: &str,
    panes: &[TmuxPane],
) -> Result<String, RoutePaneError> {
    let Some(window_target) = route_window_target_without_pane(target) else {
        return Ok(target.to_owned());
    };
    let mut candidates = panes
        .iter()
        .filter(|pane| {
            is_ls_agent_command(&pane.command)
                && candidate_window_target(&pane.target).as_deref() == Some(window_target)
        })
        .map(|pane| pane.target.clone())
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.dedup();
    match candidates.as_slice() {
        [candidate] => Ok(candidate.clone()),
        [] => Ok(target.to_owned()),
        _ => Err(RoutePaneError {
            reason: "pane_target_ambiguous".to_owned(),
            detail: format!(
                "target '{target}' resolves to multiple agent panes; refusing to guess a pane"
            ),
            hint: Some(format!("candidates: {}", candidates.join(", "))),
        }),
    }
}

fn route_pane_error(error: RoutePaneError) -> RouteResult {
    RouteResult::Error {
        reason: error.reason,
        detail: error.detail,
        hint: error.hint,
    }
}

fn prefer_pane_zero_for_ambiguous_agent<R: maw_tmux::TmuxRunner>(
    query: &str,
    target: &str,
    runner: &mut R,
) -> String {
    let Some(agent_name) = route_agent_name_from_query(query) else {
        return target.to_owned();
    };
    let Some(window_target) = route_window_target_without_pane(target) else {
        return target.to_owned();
    };
    let Ok(raw) = runner.run(
        "list-panes",
        &["-a".to_owned(), "-F".to_owned(), maw_tmux::PANE_TARGET_FORMAT.to_owned()],
    ) else {
        return target.to_owned();
    };
    let matches = maw_tmux::pane_target_candidates_from_list_panes_output(&raw)
        .into_iter()
        .filter(|candidate| {
            candidate.source == "pane-title"
                && candidate.name.eq_ignore_ascii_case(agent_name)
                && candidate_window_target(&candidate.target).as_deref() == Some(window_target)
        })
        .collect::<Vec<_>>();
    if matches.len() <= 1 {
        return target.to_owned();
    }
    matches
        .iter()
        .find(|candidate| candidate.target.rsplit_once('.').is_some_and(|(_, pane)| pane == "0"))
        .map_or_else(|| target.to_owned(), |candidate| candidate.target.clone())
}

fn route_agent_name_from_query(query: &str) -> Option<&str> {
    let query = query.trim();
    if query.is_empty() || query.eq_ignore_ascii_case("me") || query.contains('/') {
        return None;
    }
    let name = query.split_once(':').map_or(query, |(_, name)| name);
    let (name, pane_suffix) = route_split_pane_suffix(name);
    if pane_suffix.is_some() || name.is_empty() || name.bytes().all(|byte| byte.is_ascii_digit()) {
        None
    } else {
        Some(name)
    }
}

fn route_window_target_without_pane(target: &str) -> Option<&str> {
    let (_, window) = target.split_once(':')?;
    let (_, pane_suffix) = route_split_pane_suffix(window);
    pane_suffix.is_none().then_some(target)
}

fn candidate_window_target(target: &str) -> Option<String> {
    target
        .rsplit_once('.')
        .and_then(|(window, pane)| {
            (!window.is_empty() && !pane.is_empty() && pane.bytes().all(|byte| byte.is_ascii_digit()))
                .then(|| window.to_owned())
        })
}

fn route_split_pane_suffix(value: &str) -> (&str, Option<&str>) {
    if let Some((window, pane)) = value.rsplit_once('.') {
        if !window.is_empty() && !pane.is_empty() && pane.bytes().all(|byte| byte.is_ascii_digit()) {
            return (window, Some(pane));
        }
    }
    (value, None)
}

#[cfg(test)]
mod target_resolver_tests {
    use super::*;

    #[derive(Default)]
    struct FakeRunner {
        raw: String,
        calls: usize,
    }

    impl maw_tmux::TmuxRunner for FakeRunner {
        fn run(&mut self, subcommand: &str, _args: &[String]) -> Result<String, maw_tmux::TmuxError> {
            if subcommand == "list-panes" {
                self.calls += 1;
                Ok(self.raw.clone())
            } else {
                Err(maw_tmux::TmuxError::new(format!("unexpected {subcommand}")))
            }
        }
    }

    #[test]
    fn ambiguous_agent_panes_in_one_window_refuse_and_list_candidates() {
        let mut runner = FakeRunner {
            raw: [
                "%1|||codex|||81-kru32:0.2|||codex-2|||101|||/tmp|||0",
                "%2|||claude|||81-kru32:0.0|||kru32-oracle|||102|||/tmp|||0",
                "%3|||codex|||81-kru32:0.1|||codex-1|||103|||/tmp|||0",
            ]
            .join("\n"),
            ..FakeRunner::default()
        };

        let result = route_result_refuse_ambiguous_agent_panes(
            "kru32",
            RouteResult::Local { target: "81-kru32:0".to_owned() },
            &mut runner,
        );

        assert_eq!(
            result,
            RouteResult::Error {
                reason: "pane_target_ambiguous".to_owned(),
                detail: "target '81-kru32:0' resolves to multiple agent panes; refusing to guess a pane"
                    .to_owned(),
                hint: Some(
                    "candidates: 81-kru32:0.0, 81-kru32:0.1, 81-kru32:0.2".to_owned()
                ),
            }
        );
    }

    #[test]
    fn single_agent_pane_in_window_resolves_explicitly() {
        let mut runner = FakeRunner {
            raw: [
                "%1|||zsh|||81-kru32:0.0|||shell|||101|||/tmp|||0",
                "%2|||codex|||81-kru32:0.1|||codex-1|||102|||/tmp|||0",
            ]
            .join("\n"),
            ..FakeRunner::default()
        };

        let result = route_result_refuse_ambiguous_agent_panes(
            "kru32",
            RouteResult::Local {
                target: "81-kru32:0".to_owned(),
            },
            &mut runner,
        );

        assert_eq!(
            result,
            RouteResult::Local {
                target: "81-kru32:0.1".to_owned()
            }
        );
        for target in ["81-kru32:0.2", "%42"] {
            assert_eq!(
                resolve_window_agent_pane_target(target, &[]),
                Ok(target.to_owned())
            );
        }
    }

    #[test]
    fn explicit_pane_or_single_match_keeps_resolved_target() {
        let mut explicit = FakeRunner::default();
        assert_eq!(
            prefer_pane_zero_for_ambiguous_agent("81-kru32:kru32-oracle.2", "81-kru32:0.2", &mut explicit),
            "81-kru32:0.2"
        );
        assert_eq!(explicit.calls, 0);

        let mut single = FakeRunner {
            raw: "%1|||81-kru32:0.1|||kru32-oracle||||||/tmp\n".to_owned(),
            ..FakeRunner::default()
        };
        assert_eq!(
            prefer_pane_zero_for_ambiguous_agent("81-kru32:kru32-oracle", "81-kru32:0", &mut single),
            "81-kru32:0"
        );
    }
}
