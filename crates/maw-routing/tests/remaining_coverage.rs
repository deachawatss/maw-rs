use maw_routing::{resolve_target, MawConfig, ResolveResult, Session, Window};

fn window(index: u32, name: &str) -> Window {
    Window {
        index,
        name: name.to_owned(),
        active: index == 0,
    }
}

fn session(name: &str, windows: Vec<Window>) -> Session {
    Session {
        name: name.to_owned(),
        windows,
        source: None,
    }
}

#[test]
fn colon_query_without_window_uses_first_session_window() {
    let sessions = vec![session("dev", vec![window(5, "main")])];

    assert_eq!(
        resolve_target("dev:", &MawConfig::default(), &sessions),
        ResolveResult::Local {
            target: "dev:5".to_owned(),
        }
    );
}

#[test]
fn colon_numeric_window_falls_back_to_direct_target() {
    let sessions = vec![session("dev", vec![window(5, "main")])];

    assert_eq!(
        resolve_target("dev:4", &MawConfig::default(), &sessions),
        ResolveResult::Local {
            target: "dev:4".to_owned(),
        }
    );
}

#[test]
fn numeric_oracle_session_aliases_resolve_to_first_window() {
    let sessions = vec![session("101-mawjs-oracle", vec![window(2, "agent")])];

    assert_eq!(
        resolve_target("mawjs", &MawConfig::default(), &sessions),
        ResolveResult::Local {
            target: "101-mawjs-oracle:2".to_owned(),
        }
    );
}

#[test]
fn substring_window_match_requires_a_single_candidate() {
    let sessions = vec![
        session("alpha", vec![window(1, "homekeeper")]),
        session("beta", vec![window(2, "homekeeper")]),
    ];

    assert!(matches!(
        resolve_target("home", &MawConfig::default(), &sessions),
        ResolveResult::Error { .. }
    ));

    assert_eq!(
        resolve_target(
            "keeper",
            &MawConfig::default(),
            &[session("solo", vec![window(3, "homekeeper")])]
        ),
        ResolveResult::Local {
            target: "solo:3".to_owned(),
        }
    );
}
