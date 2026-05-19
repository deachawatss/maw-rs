//! Portable plugin tier and default-active policy.
//!
//! This crate mirrors the pure constants/functions in maw-js
//! `src/plugin/default-active.ts`, `src/plugin/tier.ts`, and
//! `src/plugin/manifest-constants.ts`.

/// Plugin membership tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginTier {
    Core,
    Standard,
    Extra,
}

impl PluginTier {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::Standard => "standard",
            Self::Extra => "extra",
        }
    }
}

pub const KNOWN_TIERS: &[PluginTier] = &[PluginTier::Core, PluginTier::Standard, PluginTier::Extra];
pub const DEFAULT_TIER: PluginTier = PluginTier::Core;

pub const DEFAULT_ACTIVE_PLUGINS_1500: &[&str] = &[
    "team", "fleet", "panes", "peers", "pair", "tmux", "kill", "plugin", "doctor", "inbox",
];
pub const DEFAULT_ACTIVE_PLUGINS_1500_MIGRATION: &str = "defaultActivePlugins1500";

pub const DEFAULT_ACTIVE_PLUGINS_1514: &[&str] = &["split"];
pub const DEFAULT_ACTIVE_PLUGINS_1514_MIGRATION: &str = "defaultActivePlugins1514";

pub const DEFAULT_ACTIVE_PLUGINS_1523: &[&str] = &["shellenv"];
pub const DEFAULT_ACTIVE_PLUGINS_1523_MIGRATION: &str = "defaultActivePlugins1523";

pub const DEFAULT_ACTIVE_PLUGINS_1524: &[&str] = &["completions"];
pub const DEFAULT_ACTIVE_PLUGINS_1524_MIGRATION: &str = "defaultActivePlugins1524";

pub const DEFAULT_ACTIVE_PLUGINS_1531: &[&str] =
    &["learn", "find", "talk-to", "project", "workon", "cleanup"];
pub const DEFAULT_ACTIVE_PLUGINS_1531_MIGRATION: &str = "defaultActivePlugins1531";

/// Convert manifest weight into the portable tier thresholds.
#[must_use]
pub const fn weight_to_tier(weight: i32) -> PluginTier {
    if weight < 10 {
        PluginTier::Core
    } else if weight < 50 {
        PluginTier::Standard
    } else {
        PluginTier::Extra
    }
}

#[must_use]
pub fn is_default_active_plugin(name: &str) -> bool {
    DEFAULT_ACTIVE_PLUGINS_1500.contains(&name)
}

#[must_use]
pub fn is_default_active_1514_plugin(name: &str) -> bool {
    DEFAULT_ACTIVE_PLUGINS_1514.contains(&name)
}

#[must_use]
pub fn is_default_active_1523_plugin(name: &str) -> bool {
    DEFAULT_ACTIVE_PLUGINS_1523.contains(&name)
}

#[must_use]
pub fn is_default_active_1524_plugin(name: &str) -> bool {
    DEFAULT_ACTIVE_PLUGINS_1524.contains(&name)
}

#[must_use]
pub fn is_default_active_1531_plugin(name: &str) -> bool {
    DEFAULT_ACTIVE_PLUGINS_1531.contains(&name)
}

/// Lookup a default-active policy group by migration key number.
#[must_use]
pub const fn default_active_group(key: &str) -> Option<DefaultActiveGroup> {
    match key.as_bytes() {
        b"1500" => Some(DefaultActiveGroup {
            plugins: DEFAULT_ACTIVE_PLUGINS_1500,
            migration: DEFAULT_ACTIVE_PLUGINS_1500_MIGRATION,
            includes: is_default_active_plugin,
        }),
        b"1514" => Some(DefaultActiveGroup {
            plugins: DEFAULT_ACTIVE_PLUGINS_1514,
            migration: DEFAULT_ACTIVE_PLUGINS_1514_MIGRATION,
            includes: is_default_active_1514_plugin,
        }),
        b"1523" => Some(DefaultActiveGroup {
            plugins: DEFAULT_ACTIVE_PLUGINS_1523,
            migration: DEFAULT_ACTIVE_PLUGINS_1523_MIGRATION,
            includes: is_default_active_1523_plugin,
        }),
        b"1524" => Some(DefaultActiveGroup {
            plugins: DEFAULT_ACTIVE_PLUGINS_1524,
            migration: DEFAULT_ACTIVE_PLUGINS_1524_MIGRATION,
            includes: is_default_active_1524_plugin,
        }),
        b"1531" => Some(DefaultActiveGroup {
            plugins: DEFAULT_ACTIVE_PLUGINS_1531,
            migration: DEFAULT_ACTIVE_PLUGINS_1531_MIGRATION,
            includes: is_default_active_1531_plugin,
        }),
        _ => None,
    }
}

/// One default-active migration wave.
#[derive(Clone, Copy)]
pub struct DefaultActiveGroup {
    pub plugins: &'static [&'static str],
    pub migration: &'static str,
    pub includes: fn(&str) -> bool,
}
