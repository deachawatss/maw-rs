//! XDG/legacy maw path resolver ported from maw-js `src/core/xdg.ts`.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MawXdgEnv {
    home_dir: PathBuf,
    vars: BTreeMap<String, String>,
}

impl MawXdgEnv {
    #[must_use]
    pub fn new(home_dir: impl Into<PathBuf>) -> Self {
        Self {
            home_dir: home_dir.into(),
            vars: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn with_vars(
        home_dir: impl Into<PathBuf>,
        vars: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        let mut env = Self::new(home_dir);
        for (key, value) in vars {
            env.vars.insert(key.into(), value.into());
        }
        env
    }

    #[must_use]
    pub fn var(&self, name: &str) -> Option<&str> {
        self.vars.get(name).map(String::as_str)
    }

    #[must_use]
    pub fn home_dir(&self) -> &Path {
        &self.home_dir
    }
}

#[must_use]
pub fn is_maw_xdg_enabled(env: &MawXdgEnv) -> bool {
    truthy_env(env.var("MAW_XDG"))
}

#[must_use]
pub fn maw_config_dir(env: &MawXdgEnv) -> PathBuf {
    if let Some(maw_home) = env.var("MAW_HOME") {
        return Path::new(maw_home).join("config");
    }
    if let Some(config_dir) = env.var("MAW_CONFIG_DIR") {
        return PathBuf::from(config_dir);
    }
    xdg_base(env, "XDG_CONFIG_HOME", &[".config"]).join("maw")
}

#[must_use]
pub fn maw_runtime_home_dir(env: &MawXdgEnv) -> PathBuf {
    if let Some(maw_home) = env.var("MAW_HOME") {
        return PathBuf::from(maw_home);
    }
    if is_maw_xdg_enabled(env) {
        maw_state_dir(env)
    } else {
        legacy_home(env)
    }
}

#[must_use]
pub fn maw_data_dir(env: &MawXdgEnv) -> PathBuf {
    if let Some(maw_home) = env.var("MAW_HOME") {
        return PathBuf::from(maw_home);
    }
    if let Some(data_dir) = env.var("MAW_DATA_DIR") {
        return PathBuf::from(data_dir);
    }
    if is_maw_xdg_enabled(env) {
        xdg_base(env, "XDG_DATA_HOME", &[".local", "share"]).join("maw")
    } else {
        legacy_home(env)
    }
}

#[must_use]
pub fn maw_state_dir(env: &MawXdgEnv) -> PathBuf {
    if let Some(maw_home) = env.var("MAW_HOME") {
        return PathBuf::from(maw_home);
    }
    if let Some(state_dir) = env.var("MAW_STATE_DIR") {
        return PathBuf::from(state_dir);
    }
    if is_maw_xdg_enabled(env) {
        xdg_base(env, "XDG_STATE_HOME", &[".local", "state"]).join("maw")
    } else {
        legacy_home(env)
    }
}

#[must_use]
pub fn maw_cache_dir(env: &MawXdgEnv) -> PathBuf {
    if let Some(maw_home) = env.var("MAW_HOME") {
        return PathBuf::from(maw_home);
    }
    if let Some(cache_dir) = env.var("MAW_CACHE_DIR") {
        return PathBuf::from(cache_dir);
    }
    if is_maw_xdg_enabled(env) {
        xdg_base(env, "XDG_CACHE_HOME", &[".cache"]).join("maw")
    } else {
        legacy_home(env)
    }
}

#[must_use]
pub fn maw_config_path(env: &MawXdgEnv, parts: &[&str]) -> PathBuf {
    join_parts(maw_config_dir(env), parts)
}

#[must_use]
pub fn maw_data_path(env: &MawXdgEnv, parts: &[&str]) -> PathBuf {
    join_parts(maw_data_dir(env), parts)
}

#[must_use]
pub fn maw_state_path(env: &MawXdgEnv, parts: &[&str]) -> PathBuf {
    join_parts(maw_state_dir(env), parts)
}

#[must_use]
pub fn maw_cache_path(env: &MawXdgEnv, parts: &[&str]) -> PathBuf {
    join_parts(maw_cache_dir(env), parts)
}

fn truthy_env(value: Option<&str>) -> bool {
    value.is_some_and(|value| matches!(value.to_lowercase().as_str(), "1" | "true" | "yes" | "on"))
}

fn absolute_env(env: &MawXdgEnv, name: &str) -> Option<PathBuf> {
    let value = env.var(name)?;
    let path = Path::new(value);
    path.is_absolute().then(|| path.to_path_buf())
}

fn legacy_home(env: &MawXdgEnv) -> PathBuf {
    env.home_dir().join(".maw")
}

fn xdg_base(env: &MawXdgEnv, env_name: &str, fallback: &[&str]) -> PathBuf {
    absolute_env(env, env_name)
        .unwrap_or_else(|| join_parts(env.home_dir().to_path_buf(), fallback))
}

fn join_parts(mut base: PathBuf, parts: &[&str]) -> PathBuf {
    for part in parts {
        base.push(part);
    }
    base
}
