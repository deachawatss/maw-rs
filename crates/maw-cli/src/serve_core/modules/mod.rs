//! Serve-daemon module aggregator.
//!
//! Pattern-setter for the remaining serve-* fan-out PRs:
//! 1. Add one `serve_core/modules/<name>.rs` file.
//! 2. In that file expose `<name>_lifecycle_module() -> ServecoreLifecycleModule`,
//!    `<name>_mount(router) -> Router<S>`, and `<name>_registration() -> ServecoreModuleRegistration<S>`.
//!    `views` is the one approved special case: its mount is no-op and core owns the fallback.
//! 3. Add one alphabetically sorted line to `servecore_module_registry()`.
//! 4. If the module introduces a protected route, extend `maw_auth::is_protected()` in the same PR.
//! 5. Never mount after `servecore_apply_pipeline`; all module routers must pass through default-deny.

pub mod agents;
pub mod debug;
pub mod federation;
pub mod identity;
pub mod pair;
pub mod triggers;
pub mod triggers_mutate;
pub mod views;
pub mod worktrees;
pub mod ws;
use super::{ServecoreLifecycle, ServecoreLifecycleModule};
use axum::Router;

pub struct ServecoreModuleRegistration<S>
where
    S: Clone + Send + Sync + 'static,
{
    pub lifecycle: ServecoreLifecycleModule,
    pub mount: fn(Router<S>) -> Router<S>,
}

pub fn servecore_mount_modules<S>(router: Router<S>, api_routers: &[String]) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let registrations = servecore_module_registry();
    let lifecycle = ServecoreLifecycle::servecore_from_profile(
        registrations
            .iter()
            .map(|registration| registration.lifecycle.clone())
            .collect(),
        api_routers,
    );
    let enabled = lifecycle.servecore_enabled_modules();
    registrations
        .into_iter()
        .filter(|registration| enabled.contains(&registration.lifecycle.name))
        .fold(router, |router, registration| (registration.mount)(router))
}

fn servecore_module_registry<S>() -> Vec<ServecoreModuleRegistration<S>>
where
    S: Clone + Send + Sync + 'static,
{
    vec![
        agents::agents_registration(),
        debug::debug_registration(),
        federation::federation_registration(),
        identity::identity_registration(),
        pair::pair_registration(),
        triggers::triggers_registration(),
        triggers_mutate::triggersmutate_registration(),
        views::views_registration(),
        worktrees::worktrees_registration(),
        ws::ws_registration(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;

    #[test]
    fn servecore_module_aggregator_uses_lifecycle_and_whitelist() {
        let all: Router = servecore_mount_modules(Router::new(), &[]);
        let whitelisted: Router = servecore_mount_modules(Router::new(), &["agents".to_owned()]);
        let disabled: Router = servecore_mount_modules(Router::new(), &["debug".to_owned()]);
        let _ = (all, whitelisted, disabled);
    }

    #[test]
    fn servecore_module_registry_remains_name_sorted_for_parallel_fanout() {
        let names = servecore_module_registry::<()>()
            .into_iter()
            .map(|module| module.lifecycle.name)
            .collect::<Vec<_>>();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
    }
}
