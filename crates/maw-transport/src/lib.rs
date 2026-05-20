//! Portable transport classification and failover routing.
//!
//! This crate mirrors the pure send-order behavior in maw-js
//! `src/core/transport/transport.ts` without binding to async runtime or IO.

/// Transport failure reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportFailureReason {
    Timeout,
    Unreachable,
    Auth,
    RateLimit,
    Rejected,
    ParseError,
    Unknown,
}

impl TransportFailureReason {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::Unreachable => "unreachable",
            Self::Auth => "auth",
            Self::RateLimit => "rate_limit",
            Self::Rejected => "rejected",
            Self::ParseError => "parse_error",
            Self::Unknown => "unknown",
        }
    }
}

/// Classified transport failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassifiedError {
    pub reason: TransportFailureReason,
    pub retryable: bool,
}

/// Classify common error strings into portable failure reasons.
#[must_use]
pub fn classify_error(err: Option<&str>) -> ClassifiedError {
    let Some(err) = err else {
        return ClassifiedError {
            reason: TransportFailureReason::Unknown,
            retryable: false,
        };
    };
    let msg = err.to_lowercase();
    if contains_any(&msg, &["timeout", "etimedout", "econnreset"]) {
        return ClassifiedError {
            reason: TransportFailureReason::Timeout,
            retryable: true,
        };
    }
    if contains_any(&msg, &["econnrefused", "unreachable", "enetunreach"]) {
        return ClassifiedError {
            reason: TransportFailureReason::Unreachable,
            retryable: true,
        };
    }
    if contains_any(&msg, &["401", "403", "auth", "unauthorized", "forbidden"]) {
        return ClassifiedError {
            reason: TransportFailureReason::Auth,
            retryable: false,
        };
    }
    if msg.contains("429") || msg.contains("too many") || rate_limit_like(&msg) {
        return ClassifiedError {
            reason: TransportFailureReason::RateLimit,
            retryable: true,
        };
    }
    if contains_any(&msg, &["400", "reject", "denied"]) {
        return ClassifiedError {
            reason: TransportFailureReason::Rejected,
            retryable: false,
        };
    }
    if contains_any(&msg, &["parse", "json", "syntax"]) {
        return ClassifiedError {
            reason: TransportFailureReason::ParseError,
            retryable: false,
        };
    }
    ClassifiedError {
        reason: TransportFailureReason::Unknown,
        retryable: false,
    }
}

/// Result of a routed send attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportResult {
    pub ok: bool,
    pub via: String,
    pub reason: Option<TransportFailureReason>,
    pub retryable: bool,
}

impl TransportResult {
    #[must_use]
    pub fn success(via: impl Into<String>) -> Self {
        Self {
            ok: true,
            via: via.into(),
            reason: None,
            retryable: false,
        }
    }

    #[must_use]
    pub fn failure(
        via: impl Into<String>,
        reason: TransportFailureReason,
        retryable: bool,
    ) -> Self {
        Self {
            ok: false,
            via: via.into(),
            reason: Some(reason),
            retryable,
        }
    }
}

/// Destination metadata for transport selection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TransportTarget {
    pub oracle: String,
    pub host: Option<String>,
    pub tmux_target: Option<String>,
}

/// Window shape used by local tmux target resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TmuxTransportWindow {
    pub index: u32,
    pub name: String,
    pub active: bool,
}

/// Session shape used by local tmux target resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TmuxTransportSession {
    pub name: String,
    pub windows: Vec<TmuxTransportWindow>,
}

/// Minimal portable transport trait.
pub trait Transport {
    fn name(&self) -> &str;
    fn connected(&self) -> bool;
    fn can_reach(&self, target: &TransportTarget) -> bool;
    /// Send a message through this transport.
    ///
    /// # Errors
    ///
    /// Returns an error string when the transport attempted delivery but failed.
    /// The router classifies that error to decide whether to fail over.
    fn send(&mut self, target: &TransportTarget, message: &str, from: &str)
        -> Result<bool, String>;
}

/// Ordered transport router. First successful reachable transport wins.
#[derive(Default)]
pub struct TransportRouter<T> {
    transports: Vec<T>,
}

impl<T> TransportRouter<T>
where
    T: Transport,
{
    #[must_use]
    pub const fn new() -> Self {
        Self {
            transports: Vec::new(),
        }
    }

    pub fn register(&mut self, transport: T) {
        self.transports.push(transport);
    }

    pub fn send(&mut self, target: &TransportTarget, message: &str, from: &str) -> TransportResult {
        for transport in &mut self.transports {
            if !transport.connected() || !transport.can_reach(target) {
                continue;
            }

            match transport.send(target, message, from) {
                Ok(true) => return TransportResult::success(transport.name()),
                Ok(false) => {}
                Err(err) => {
                    let classified = classify_error(Some(&err));
                    if !classified.retryable {
                        return TransportResult::failure(
                            transport.name(),
                            classified.reason,
                            classified.retryable,
                        );
                    }
                }
            }
        }
        TransportResult::failure("none", TransportFailureReason::Unreachable, false)
    }
}

/// Side-effect seam for the local tmux transport.
pub trait TmuxTransportIo {
    /// Send a message to a concrete tmux target.
    ///
    /// # Errors
    ///
    /// Returns an implementation-specific error string when tmux rejects delivery.
    fn send_to_tmux(&mut self, target: &str, message: &str) -> Result<(), String>;

    /// List local tmux sessions for oracle-name resolution.
    ///
    /// # Errors
    ///
    /// Returns an implementation-specific error string when session listing fails.
    fn list_tmux_sessions(&mut self) -> Result<Vec<TmuxTransportSession>, String>;

    /// Resolve an oracle query to a tmux target from already-listed sessions.
    fn find_tmux_window(
        &mut self,
        sessions: &[TmuxTransportSession],
        query: &str,
    ) -> Option<String>;
}

/// Portable local fast-path tmux transport.
pub struct TmuxLocalTransport<Io> {
    io: Io,
    connected: bool,
    message_handlers: usize,
    presence_handlers: usize,
    feed_handlers: usize,
}

impl<Io> TmuxLocalTransport<Io> {
    #[must_use]
    pub const fn new(io: Io) -> Self {
        Self {
            io,
            connected: false,
            message_handlers: 0,
            presence_handlers: 0,
            feed_handlers: 0,
        }
    }

    #[must_use]
    pub const fn connected(&self) -> bool {
        self.connected
    }

    pub const fn connect(&mut self) {
        self.connected = true;
    }

    pub const fn disconnect(&mut self) {
        self.connected = false;
    }

    pub const fn on_message(&mut self) {
        self.message_handlers += 1;
    }

    pub const fn on_presence(&mut self) {
        self.presence_handlers += 1;
    }

    pub const fn on_feed(&mut self) {
        self.feed_handlers += 1;
    }

    #[must_use]
    pub const fn handler_counts(&self) -> (usize, usize, usize) {
        (
            self.message_handlers,
            self.presence_handlers,
            self.feed_handlers,
        )
    }

    pub const fn publish_presence(&self) {}

    pub const fn publish_feed(&self) {}
}

impl<Io> TmuxLocalTransport<Io>
where
    Io: TmuxTransportIo,
{
    #[must_use]
    pub fn name(&self) -> &'static str {
        "tmux"
    }

    #[must_use]
    pub fn can_reach(&self, target: &TransportTarget) -> bool {
        is_local_host(target.host.as_deref())
    }

    /// Send using explicit `tmux_target` or by scanning sessions and resolving the oracle name.
    pub fn send(&mut self, target: &TransportTarget, message: &str) -> bool {
        if !self.can_reach(target) {
            return false;
        }
        let tmux_target = if let Some(tmux_target) = &target.tmux_target {
            tmux_target.clone()
        } else {
            let Ok(sessions) = self.io.list_tmux_sessions() else {
                return false;
            };
            let Some(resolved) = self.io.find_tmux_window(&sessions, &target.oracle) else {
                return false;
            };
            resolved
        };
        self.io.send_to_tmux(&tmux_target, message).is_ok()
    }

    #[must_use]
    pub const fn io(&self) -> &Io {
        &self.io
    }
}

fn is_local_host(host: Option<&str>) -> bool {
    matches!(host, None | Some("local" | "localhost"))
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn rate_limit_like(msg: &str) -> bool {
    msg.contains("rate") && msg.contains("limit")
}

#[cfg(test)]
mod tmux_transport_tests {
    use super::*;

    #[derive(Default)]
    struct FakeTmuxIo {
        sends: Vec<(String, String)>,
        scanned: bool,
        sessions: Vec<TmuxTransportSession>,
        queries: Vec<String>,
        find_result: Option<String>,
        send_error: bool,
    }

    impl TmuxTransportIo for FakeTmuxIo {
        fn send_to_tmux(&mut self, target: &str, message: &str) -> Result<(), String> {
            if self.send_error {
                return Err("tmux rejected".to_owned());
            }
            self.sends.push((target.to_owned(), message.to_owned()));
            Ok(())
        }

        fn list_tmux_sessions(&mut self) -> Result<Vec<TmuxTransportSession>, String> {
            self.scanned = true;
            Ok(self.sessions.clone())
        }

        fn find_tmux_window(
            &mut self,
            sessions: &[TmuxTransportSession],
            query: &str,
        ) -> Option<String> {
            assert_eq!(sessions, self.sessions.as_slice());
            self.queries.push(query.to_owned());
            self.find_result.clone()
        }
    }

    fn sample_sessions() -> Vec<TmuxTransportSession> {
        vec![TmuxTransportSession {
            name: "47-mawjs".to_owned(),
            windows: vec![
                TmuxTransportWindow {
                    index: 0,
                    name: "mawjs-oracle".to_owned(),
                    active: true,
                },
                TmuxTransportWindow {
                    index: 1,
                    name: "mawjs-codex".to_owned(),
                    active: false,
                },
            ],
        }]
    }

    #[test]
    fn tmux_transport_tracks_local_lifecycle_and_reachability() {
        let mut transport = TmuxLocalTransport::new(FakeTmuxIo::default());
        assert_eq!(transport.name(), "tmux");
        assert!(!transport.connected());
        transport.connect();
        assert!(transport.connected());
        transport.disconnect();
        assert!(!transport.connected());

        assert!(transport.can_reach(&TransportTarget {
            oracle: "mawjs".to_owned(),
            host: None,
            tmux_target: None,
        }));
        assert!(transport.can_reach(&TransportTarget {
            oracle: "mawjs".to_owned(),
            host: Some("local".to_owned()),
            tmux_target: None,
        }));
        assert!(transport.can_reach(&TransportTarget {
            oracle: "mawjs".to_owned(),
            host: Some("localhost".to_owned()),
            tmux_target: None,
        }));
        assert!(!transport.can_reach(&TransportTarget {
            oracle: "mawjs".to_owned(),
            host: Some("m5".to_owned()),
            tmux_target: None,
        }));
    }

    #[test]
    fn tmux_transport_uses_explicit_target_without_scanning() {
        let mut transport = TmuxLocalTransport::new(FakeTmuxIo::default());
        assert!(transport.send(
            &TransportTarget {
                oracle: "ignored".to_owned(),
                host: None,
                tmux_target: Some("47-mawjs:1".to_owned()),
            },
            "hello",
        ));
        assert!(!transport.io().scanned);
        assert_eq!(
            transport.io().sends,
            vec![("47-mawjs:1".to_owned(), "hello".to_owned())]
        );
    }

    #[test]
    fn tmux_transport_resolves_local_oracle_through_session_scan() {
        let io = FakeTmuxIo {
            sessions: sample_sessions(),
            find_result: Some("47-mawjs:1".to_owned()),
            ..FakeTmuxIo::default()
        };
        let mut transport = TmuxLocalTransport::new(io);
        assert!(transport.send(
            &TransportTarget {
                oracle: "mawjs-codex".to_owned(),
                host: None,
                tmux_target: None,
            },
            "ping",
        ));
        assert!(transport.io().scanned);
        assert_eq!(transport.io().queries, vec!["mawjs-codex".to_owned()]);
        assert_eq!(
            transport.io().sends,
            vec![("47-mawjs:1".to_owned(), "ping".to_owned())]
        );
    }

    #[test]
    fn tmux_transport_returns_false_for_remote_unresolved_and_throwing_paths() {
        let mut remote = TmuxLocalTransport::new(FakeTmuxIo {
            sessions: sample_sessions(),
            ..FakeTmuxIo::default()
        });
        assert!(!remote.send(
            &TransportTarget {
                oracle: "mawjs".to_owned(),
                host: Some("remote".to_owned()),
                tmux_target: None,
            },
            "nope",
        ));
        assert!(remote.io().sends.is_empty());

        let mut unresolved = TmuxLocalTransport::new(FakeTmuxIo {
            sessions: sample_sessions(),
            find_result: None,
            ..FakeTmuxIo::default()
        });
        assert!(!unresolved.send(
            &TransportTarget {
                oracle: "missing".to_owned(),
                host: None,
                tmux_target: None,
            },
            "nope",
        ));

        let mut throwing = TmuxLocalTransport::new(FakeTmuxIo {
            sessions: sample_sessions(),
            find_result: Some("47-mawjs:1".to_owned()),
            send_error: true,
            ..FakeTmuxIo::default()
        });
        assert!(!throwing.send(
            &TransportTarget {
                oracle: "mawjs".to_owned(),
                host: None,
                tmux_target: None,
            },
            "nope",
        ));
        assert!(throwing.io().sends.is_empty());
    }

    #[test]
    fn tmux_transport_accepts_handlers_and_ignores_publish_hooks() {
        let mut transport = TmuxLocalTransport::new(FakeTmuxIo::default());
        transport.on_message();
        transport.on_presence();
        transport.on_feed();
        assert_eq!(transport.handler_counts(), (1, 1, 1));
        transport.publish_presence();
        transport.publish_feed();
    }
}
