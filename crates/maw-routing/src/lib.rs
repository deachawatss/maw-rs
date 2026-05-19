//! Portable target routing resolver.
//!
//! This crate mirrors the pure, sync behavior in maw-js `src/core/routing.ts`
//! that is covered by `test/spec/routing.fixtures.json`.

use std::collections::HashMap;

/// Tmux window metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    pub index: u32,
    pub name: String,
    pub active: bool,
}

/// Tmux session metadata. `source` is `None`/`local` for writable local sessions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub name: String,
    pub windows: Vec<Window>,
    pub source: Option<String>,
}

/// Named peer config entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedPeer {
    pub name: String,
    pub url: String,
}

/// Minimal config surface needed by the portable resolver.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MawConfig {
    pub node: Option<String>,
    pub named_peers: Vec<NamedPeer>,
    pub peers: Vec<String>,
    pub agents: HashMap<String, String>,
}

/// Routing resolution result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveResult {
    Local {
        target: String,
    },
    Peer {
        peer_url: String,
        target: String,
        node: String,
    },
    SelfNode {
        target: String,
    },
    Error {
        reason: String,
        detail: String,
        hint: Option<String>,
    },
}

/// Resolve a user query to a local target, peer target, self-node target, or error.
#[allow(clippy::too_many_lines)]
#[must_use]
pub fn resolve_target(query: &str, config: &MawConfig, sessions: &[Session]) -> ResolveResult {
    if query.is_empty() {
        return error(
            "empty_query",
            "no target specified",
            Some("usage: maw hey <agent> <message>"),
        );
    }

    let writable: Vec<Session> = sessions
        .iter()
        .filter(|session| {
            !session.name.ends_with("-view")
                && session
                    .source
                    .as_deref()
                    .is_none_or(|source| source == "local")
        })
        .cloned()
        .collect();
    let self_node = config.node.as_deref().unwrap_or("local");

    if !query.contains(':') {
        if let Some(result) =
            resolve_session_alias_window_target(query, &writable, RouteType::Local)
        {
            return result;
        }
    }

    if let Some(local_target) = find_window(&writable, query) {
        return ResolveResult::Local {
            target: local_target,
        };
    }

    if query.contains(':') && !query.contains('/') {
        let (node_name, agent_name) = query.split_once(':').unwrap_or(("", ""));
        if node_name.is_empty() || agent_name.is_empty() {
            return error(
                "empty_node_or_agent",
                format!("invalid format: '{query}'"),
                Some("use node:agent format (e.g. mba:homekeeper)"),
            );
        }

        if node_name == self_node || node_name == "local" {
            if let Some(result) =
                resolve_session_alias_window_target(agent_name, &writable, RouteType::SelfNode)
            {
                return result;
            }
            if let Some(self_target) = find_window(&writable, agent_name) {
                return ResolveResult::SelfNode {
                    target: self_target,
                };
            }
            return error(
                "self_not_running",
                format!("'{agent_name}' not found in local sessions on {self_node}"),
                Some(format!("maw wake {agent_name}")),
            );
        }

        if let Some(peer_url) = find_peer_url(node_name, config) {
            return ResolveResult::Peer {
                peer_url,
                target: agent_name.to_owned(),
                node: node_name.to_owned(),
            };
        }

        return error(
            "unknown_node",
            format!("node '{node_name}' not in namedPeers or peers"),
            Some("add to maw.config.json namedPeers"),
        );
    }

    let stripped_query = query.strip_suffix("-oracle").unwrap_or(query);
    let agent_node = config
        .agents
        .get(query)
        .or_else(|| config.agents.get(stripped_query));

    if let Some(agent_node) = agent_node {
        if agent_node == self_node {
            return error(
                "self_not_running",
                format!("'{query}' mapped to {self_node} (local) but not found in sessions"),
                Some(format!("maw wake {query}")),
            );
        }
        if let Some(peer_url) = find_peer_url(agent_node, config) {
            return ResolveResult::Peer {
                peer_url,
                target: query.to_owned(),
                node: agent_node.clone(),
            };
        }
        return error(
            "no_peer_url",
            format!("'{query}' mapped to node '{agent_node}' but no URL found"),
            Some(format!("add {agent_node} to maw.config.json namedPeers")),
        );
    }

    error(
        "not_found",
        format!("'{query}' not in local sessions or agents map"),
        Some("check: maw ls"),
    )
}

fn find_peer_url(node_name: &str, config: &MawConfig) -> Option<String> {
    config
        .named_peers
        .iter()
        .find(|peer| peer.name == node_name)
        .map(|peer| peer.url.clone())
        .or_else(|| {
            config
                .peers
                .iter()
                .find(|peer| peer.contains(node_name))
                .cloned()
        })
}

#[derive(Debug, Clone, Copy)]
enum RouteType {
    Local,
    SelfNode,
}

fn route_target(route_type: RouteType, target: String) -> ResolveResult {
    match route_type {
        RouteType::Local => ResolveResult::Local { target },
        RouteType::SelfNode => ResolveResult::SelfNode { target },
    }
}

fn resolve_session_alias_window_target(
    query: &str,
    writable: &[Session],
    route_type: RouteType,
) -> Option<ResolveResult> {
    if query.trim().to_lowercase().ends_with("-oracle") {
        return None;
    }

    let wanted = session_alias_names(query);
    if wanted.is_empty() {
        return None;
    }
    let wanted_lower: Vec<String> = wanted.iter().map(|name| name.to_lowercase()).collect();
    let mut matches: Vec<Session> = writable
        .iter()
        .filter(|session| {
            session_alias_names(&session.name)
                .iter()
                .any(|name| wanted_lower.contains(&name.to_lowercase()))
        })
        .cloned()
        .collect();

    if matches.is_empty() {
        return None;
    }

    if matches.len() > 1 {
        let normalized_query = query.trim().to_lowercase();
        let exact_unnumbered: Vec<Session> = matches
            .iter()
            .filter(|session| {
                strip_numeric_fleet_prefix(&session.name).to_lowercase() == normalized_query
            })
            .cloned()
            .collect();
        if exact_unnumbered.len() == 1 {
            matches = exact_unnumbered;
        }
    }

    if matches.len() > 1 {
        return Some(error(
            "session_alias_ambiguous",
            format!("'{query}' matches multiple local sessions; refusing to guess a window"),
            Some(format!(
                "candidates: {}",
                matches
                    .iter()
                    .map(|s| s.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        ));
    }

    let session = &matches[0];
    if let Some(named_target) = find_named_fleet_window(session, query) {
        return Some(route_target(route_type, named_target));
    }

    if session.windows.len() == 1 {
        return Some(route_target(
            route_type,
            format!("{}:{}", session.name, session.windows[0].index),
        ));
    }

    let candidate_names = fleet_window_candidate_names(query);
    let candidates = session
        .windows
        .iter()
        .map(|window| format!("{}:{} ({})", session.name, window.index, window.name))
        .collect::<Vec<_>>()
        .join(", ");
    Some(error(
        "session_window_not_found",
        format!(
            "'{query}' matched local session '{}', but no window named {} was found; refusing to default to the first window",
            session.name,
            quoted_or(&candidate_names)
        ),
        Some(format!("candidates: {candidates}")),
    ))
}

fn find_named_fleet_window(session: &Session, query: &str) -> Option<String> {
    for name in fleet_window_candidate_names(query) {
        if let Some(window) = session
            .windows
            .iter()
            .find(|window| window.name.eq_ignore_ascii_case(&name))
        {
            return Some(format!("{}:{}", session.name, window.index));
        }
    }
    None
}

fn fleet_window_candidate_names(query: &str) -> Vec<String> {
    let raw = query.trim();
    let stripped = raw.strip_suffix("-oracle").unwrap_or(raw);
    let unnumbered = strip_numeric_fleet_prefix(raw);
    let stripped_unnumbered = unnumbered.strip_suffix("-oracle").unwrap_or(unnumbered);
    let mut names = Vec::new();
    if !raw.is_empty() {
        names.push(raw.to_owned());
    }
    if stripped != raw {
        names.push(stripped.to_owned());
    }
    if unnumbered != raw {
        names.push(unnumbered.to_owned());
    }
    if stripped_unnumbered != unnumbered {
        names.push(stripped_unnumbered.to_owned());
    }
    if !stripped.is_empty() {
        names.push(format!("{stripped}-oracle"));
    }
    if !raw.to_lowercase().ends_with("-oracle") && !raw.is_empty() {
        names.push(format!("{raw}-oracle"));
    }
    if !stripped_unnumbered.is_empty() {
        names.push(format!("{stripped_unnumbered}-oracle"));
    }
    unique_strings(names)
}

fn session_alias_names(name: &str) -> Vec<String> {
    let raw = name.trim();
    let unnumbered = strip_numeric_fleet_prefix(raw);
    unique_strings(
        [
            nonempty(raw).map(str::to_owned),
            raw.strip_suffix("-oracle").map(str::to_owned),
            nonempty(unnumbered).map(str::to_owned),
            unnumbered.strip_suffix("-oracle").map(str::to_owned),
        ]
        .into_iter()
        .flatten(),
    )
}

fn find_window(sessions: &[Session], query: &str) -> Option<String> {
    let q = query.to_lowercase();

    if query.contains(':') {
        let (sess_part, raw_win_part) = q.split_once(':').unwrap_or(("", ""));
        let (win_part, pane_suffix) = split_pane_suffix(raw_win_part);
        if let Some(session) = match_session(sessions, sess_part, true) {
            if win_part.is_empty() {
                if let Some(window) = session.windows.first() {
                    return Some(format!("{}:{}", session.name, window.index));
                }
            } else if let Some(window) = session
                .windows
                .iter()
                .find(|window| window.name.to_lowercase().contains(win_part))
            {
                return Some(format!("{}:{}{pane_suffix}", session.name, window.index));
            }
        }
    }

    let exact_sessions: Vec<String> = sessions
        .iter()
        .filter_map(|session| {
            let window = session.windows.first()?;
            let name = session.name.to_lowercase();
            (name == q || strip_numeric_fleet_prefix(&name) == q)
                .then(|| format!("{}:{}", session.name, window.index))
        })
        .collect();
    if exact_sessions.len() == 1 {
        return exact_sessions.first().cloned();
    }
    if exact_sessions.len() > 1 {
        return None;
    }

    let exact_windows = unique_strings(sessions.iter().flat_map(|session| {
        let q = q.clone();
        session
            .windows
            .iter()
            .filter(move |window| window.name.eq_ignore_ascii_case(&q))
            .map(|window| format!("{}:{}", session.name, window.index))
    }));
    if exact_windows.len() == 1 {
        return exact_windows.first().cloned();
    }
    if exact_windows.len() > 1 {
        return None;
    }

    let substring_matches = unique_strings(sessions.iter().flat_map(|session| {
        let mut matches = Vec::new();
        for window in &session.windows {
            if window.name.to_lowercase().contains(&q) {
                matches.push(format!("{}:{}", session.name, window.index));
            }
        }
        if session.name.to_lowercase().contains(&q) {
            if let Some(window) = session.windows.first() {
                matches.push(format!("{}:{}", session.name, window.index));
            }
        }
        matches
    }));
    if substring_matches.len() == 1 {
        return substring_matches.first().cloned();
    }
    if substring_matches.len() > 1 {
        return None;
    }

    if query.contains(':') {
        let lower_query = query.to_lowercase();
        let (sess_part, win_part) = lower_query.split_once(':').unwrap_or(("", ""));
        let session_exists = match_session(sessions, sess_part, true).is_some();
        if !session_exists {
            return None;
        }
        if win_part.is_empty() || numeric_window_or_pane(win_part) {
            return Some(query.to_owned());
        }
    }

    None
}

fn match_session<'a>(sessions: &'a [Session], part: &str, strict: bool) -> Option<&'a Session> {
    let p = part.to_lowercase();
    if p.is_empty() {
        return None;
    }
    sessions
        .iter()
        .find(|session| session.name.to_lowercase() == p)
        .or_else(|| {
            sessions
                .iter()
                .find(|session| strip_numeric_fleet_prefix(&session.name.to_lowercase()) == p)
        })
        .or_else(|| {
            (!strict)
                .then(|| {
                    sessions
                        .iter()
                        .find(|session| session.name.to_lowercase().contains(&p))
                })
                .flatten()
        })
}

fn split_pane_suffix(raw_win_part: &str) -> (&str, String) {
    if let Some((win, pane)) = raw_win_part.rsplit_once('.') {
        if !win.is_empty() && !pane.is_empty() && pane.bytes().all(|byte| byte.is_ascii_digit()) {
            return (win, format!(".{pane}"));
        }
    }
    (raw_win_part, String::new())
}

fn numeric_window_or_pane(value: &str) -> bool {
    let Some((window, pane)) = value.split_once('.') else {
        return !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit());
    };
    !window.is_empty()
        && !pane.is_empty()
        && window.bytes().all(|byte| byte.is_ascii_digit())
        && pane.bytes().all(|byte| byte.is_ascii_digit())
}

fn strip_numeric_fleet_prefix(name: &str) -> &str {
    let Some((prefix, rest)) = name.split_once('-') else {
        return name;
    };
    if !prefix.is_empty() && prefix.bytes().all(|byte| byte.is_ascii_digit()) {
        rest
    } else {
        name
    }
}

fn nonempty(value: &str) -> Option<&str> {
    (!value.is_empty()).then_some(value)
}

fn unique_strings<I, S>(values: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut out = Vec::new();
    for value in values {
        let value = value.into();
        if !out.contains(&value) {
            out.push(value);
        }
    }
    out
}

fn quoted_or(names: &[String]) -> String {
    names
        .iter()
        .map(|name| format!("'{name}'"))
        .collect::<Vec<_>>()
        .join(" or ")
}

fn error(
    reason: impl Into<String>,
    detail: impl Into<String>,
    hint: Option<impl Into<String>>,
) -> ResolveResult {
    ResolveResult::Error {
        reason: reason.into(),
        detail: detail.into(),
        hint: hint.map(Into::into),
    }
}
