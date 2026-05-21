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
