use maw_transport::{
    classify_error, ClassifiedError, HttpFederationTransport, HttpPostResult, HttpTransportConfig,
    HttpTransportIo, TmuxTransportSession, TmuxTransportWindow, Transport, TransportFailureReason,
    TransportResult, TransportRouter, TransportSession, TransportTarget,
};
use serde::Deserialize;
use std::{cell::RefCell, rc::Rc};

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum Fixture {
    #[serde(rename = "classifyError")]
    ClassifyError {
        name: String,
        error: Option<String>,
        expected: ExpectedClassifiedError,
    },
    Send {
        name: String,
        target: Option<FixtureTarget>,
        message: Option<String>,
        from: Option<String>,
        transports: Vec<FixtureTransport>,
        expected: ExpectedSend,
    },
}

#[derive(Debug, Deserialize)]
struct FixtureTarget {
    oracle: String,
    host: Option<String>,
    #[serde(rename = "tmuxTarget")]
    tmux_target: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FixtureTransport {
    name: String,
    connected: Option<bool>,
    can_reach: Option<bool>,
    send: Option<SendAction>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum SendAction {
    Ok,
    False,
    Throw { error: String },
}

#[derive(Debug, Deserialize)]
struct ExpectedClassifiedError {
    reason: String,
    retryable: bool,
}

#[derive(Debug, Deserialize)]
struct ExpectedSend {
    result: ExpectedTransportResult,
    sent: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ExpectedTransportResult {
    ok: bool,
    via: String,
    reason: Option<String>,
    retryable: bool,
}

#[derive(Default)]
struct LocalIo {
    list_error: bool,
    resolve: Option<String>,
    send_ok: bool,
}

impl maw_transport::TmuxTransportIo for LocalIo {
    fn send_to_tmux(&mut self, _: &str, _: &str) -> Result<(), String> {
        if self.send_ok {
            Ok(())
        } else {
            Err("send failed".to_owned())
        }
    }

    fn list_tmux_sessions(&mut self) -> Result<Vec<TmuxTransportSession>, String> {
        if self.list_error {
            Err("list failed".to_owned())
        } else {
            Ok(vec![TmuxTransportSession {
                name: "local".to_owned(),
                windows: vec![TmuxTransportWindow {
                    index: 1,
                    name: "mawjs".to_owned(),
                    active: true,
                }],
            }])
        }
    }

    fn find_tmux_window(&mut self, _: &[TmuxTransportSession], _: &str) -> Option<String> {
        self.resolve.clone()
    }
}

struct FixtureTransportRuntime {
    fixture: FixtureTransport,
    sent: Rc<RefCell<Vec<String>>>,
}

impl Transport for FixtureTransportRuntime {
    fn name(&self) -> &str {
        &self.fixture.name
    }

    fn connected(&self) -> bool {
        self.fixture.connected.unwrap_or(true)
    }

    fn can_reach(&self, _target: &TransportTarget) -> bool {
        self.fixture.can_reach.unwrap_or(true)
    }

    fn send(
        &mut self,
        _target: &TransportTarget,
        _message: &str,
        _from: &str,
    ) -> Result<bool, String> {
        self.sent.borrow_mut().push(self.fixture.name.clone());
        match self.fixture.send.as_ref().unwrap_or(&SendAction::Ok) {
            SendAction::Ok => Ok(true),
            SendAction::False => Ok(false),
            SendAction::Throw { error } => Err(error.clone()),
        }
    }
}

impl From<FixtureTarget> for TransportTarget {
    fn from(target: FixtureTarget) -> Self {
        Self {
            oracle: target.oracle,
            host: target.host,
            tmux_target: target.tmux_target,
        }
    }
}

fn reason_from_str(reason: &str) -> TransportFailureReason {
    match reason {
        "timeout" => TransportFailureReason::Timeout,
        "unreachable" => TransportFailureReason::Unreachable,
        "auth" => TransportFailureReason::Auth,
        "rate_limit" => TransportFailureReason::RateLimit,
        "rejected" => TransportFailureReason::Rejected,
        "parse_error" => TransportFailureReason::ParseError,
        "unknown" => TransportFailureReason::Unknown,
        other => panic!("unknown fixture reason: {other}"),
    }
}

fn expected_classified(expected: &ExpectedClassifiedError) -> ClassifiedError {
    ClassifiedError {
        reason: reason_from_str(&expected.reason),
        retryable: expected.retryable,
    }
}

fn expected_result(expected: ExpectedTransportResult) -> TransportResult {
    TransportResult {
        ok: expected.ok,
        via: expected.via,
        reason: expected.reason.as_deref().map(reason_from_str),
        retryable: expected.retryable,
    }
}

#[test]
fn transport_router_fixtures_match_maw_js_portable_spec() {
    let fixtures: Vec<Fixture> =
        serde_json::from_str(include_str!("fixtures/transport-router.fixtures.json"))
            .expect("valid transport router fixture json");

    for fixture in fixtures {
        match fixture {
            Fixture::ClassifyError {
                name,
                error,
                expected,
            } => {
                assert_eq!(
                    classify_error(error.as_deref()),
                    expected_classified(&expected),
                    "{name}"
                );
            }
            Fixture::Send {
                name,
                target,
                message,
                from,
                transports,
                expected,
            } => {
                let sent = Rc::new(RefCell::new(Vec::new()));
                let mut router = TransportRouter::new();
                for transport in transports {
                    router.register(FixtureTransportRuntime {
                        fixture: transport,
                        sent: Rc::clone(&sent),
                    });
                }
                let target = target.map_or_else(
                    || TransportTarget {
                        oracle: "neo".to_owned(),
                        host: None,
                        tmux_target: Some("neo:1".to_owned()),
                    },
                    Into::into,
                );
                let result = router.send(
                    &target,
                    message.as_deref().unwrap_or("hello"),
                    from.as_deref().unwrap_or("codex"),
                );
                assert_eq!(result, expected_result(expected.result), "{name}");
                assert_eq!(*sent.borrow(), expected.sent, "sent order: {name}");
            }
        }
    }
}

struct UnresolvedThenHitIo;

impl HttpTransportIo for UnresolvedThenHitIo {
    fn list_local_sessions(&mut self) -> Result<Vec<TmuxTransportSession>, String> {
        Ok(Vec::new())
    }

    fn get_all_sessions(
        &mut self,
        _: &[TmuxTransportSession],
    ) -> Result<Vec<TransportSession>, String> {
        Ok(vec![
            remote_session("first", "http://first", 1),
            remote_session("second", "http://second", 2),
        ])
    }

    fn find_target_window(&mut self, sessions: &[TransportSession], _: &str) -> Option<String> {
        (sessions[0].name == "second").then(|| "second:2".to_owned())
    }

    fn send_peer_keys(&mut self, source: &str, target: &str, _: &str) -> Result<bool, String> {
        assert_eq!(source, "http://second");
        assert_eq!(target, "second:2");
        Ok(true)
    }

    fn post_peer_feed(
        &mut self,
        _: &str,
        _: &str,
        _: &str,
        _: u64,
    ) -> Result<HttpPostResult, String> {
        Ok(HttpPostResult {
            ok: true,
            status: 200,
        })
    }

    fn timeout_for(&self, _: &str) -> u64 {
        1
    }
}

fn remote_session(name: &str, source: &str, index: u32) -> TransportSession {
    TransportSession {
        name: name.to_owned(),
        source: Some(source.to_owned()),
        windows: vec![TmuxTransportWindow {
            index,
            name: "mawjs".to_owned(),
            active: false,
        }],
    }
}

#[test]
fn http_transport_continues_after_unresolved_remote_window() {
    let config = HttpTransportConfig {
        peers: vec!["http://peer".to_owned()],
        self_host: String::new(),
    };
    let mut transport = HttpFederationTransport::new(config, UnresolvedThenHitIo);

    assert!(transport.send(
        &TransportTarget {
            oracle: "mawjs".to_owned(),
            host: Some("remote".to_owned()),
            tmux_target: None,
        },
        "hello"
    ));
}

#[test]
fn remaining_transport_edges_cover_unknown_and_local_failure_paths() {
    assert_eq!(
        classify_error(Some("opaque failure")),
        ClassifiedError {
            reason: TransportFailureReason::Unknown,
            retryable: false,
        }
    );

    let local_target = TransportTarget {
        oracle: "mawjs".to_owned(),
        host: Some("localhost".to_owned()),
        tmux_target: None,
    };
    let remote_target = TransportTarget {
        oracle: "mawjs".to_owned(),
        host: Some("remote".to_owned()),
        tmux_target: Some("s:1".to_owned()),
    };

    let mut list_error = maw_transport::TmuxLocalTransport::new(LocalIo {
        list_error: true,
        ..LocalIo::default()
    });
    assert!(!list_error.send(&local_target, "hello"));

    let mut unresolved = maw_transport::TmuxLocalTransport::new(LocalIo::default());
    assert!(!unresolved.send(&local_target, "hello"));

    let mut send_error = maw_transport::TmuxLocalTransport::new(LocalIo {
        resolve: Some("local:1".to_owned()),
        send_ok: false,
        ..LocalIo::default()
    });
    assert!(!send_error.send(&local_target, "hello"));
    assert!(!send_error.send(&remote_target, "hello"));

    let mut explicit = maw_transport::TmuxLocalTransport::new(LocalIo {
        send_ok: true,
        ..LocalIo::default()
    });
    assert!(explicit.send(
        &TransportTarget {
            tmux_target: Some("local:1".to_owned()),
            ..local_target
        },
        "hello"
    ));
}
