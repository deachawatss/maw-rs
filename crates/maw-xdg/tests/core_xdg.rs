use maw_xdg::{
    is_maw_xdg_enabled, maw_cache_dir, maw_cache_path, maw_config_dir, maw_config_path,
    maw_data_dir, maw_data_path, maw_runtime_home_dir, maw_state_dir, maw_state_path, MawXdgEnv,
};
use std::path::PathBuf;

fn home() -> PathBuf {
    PathBuf::from("/home/tester")
}

fn env(vars: &[(&str, &str)]) -> MawXdgEnv {
    MawXdgEnv::with_vars(home(), vars.iter().copied())
}

#[test]
fn keeps_legacy_maw_home_defaults_until_maw_xdg_is_enabled() {
    let env = env(&[]);

    assert!(!is_maw_xdg_enabled(&env));
    assert_eq!(maw_runtime_home_dir(&env), home().join(".maw"));
    assert_eq!(maw_data_dir(&env), home().join(".maw"));
    assert_eq!(maw_state_dir(&env), home().join(".maw"));
    assert_eq!(maw_cache_dir(&env), home().join(".maw"));
    assert_eq!(maw_config_dir(&env), home().join(".config").join("maw"));
    assert_eq!(
        maw_data_path(&env, &["plugins"]),
        home().join(".maw").join("plugins")
    );
    assert_eq!(
        maw_state_path(&env, &["peers.json"]),
        home().join(".maw").join("peers.json")
    );
    assert_eq!(
        maw_cache_path(&env, &["registry-cache.json"]),
        home().join(".maw").join("registry-cache.json")
    );
    assert_eq!(
        maw_config_path(&env, &["maw.config.json"]),
        home().join(".config").join("maw").join("maw.config.json")
    );
}

#[test]
fn maw_xdg_flips_runtime_data_state_cache_to_xdg_bases() {
    let env = env(&[
        ("MAW_XDG", "yes"),
        ("XDG_DATA_HOME", "/xdg-data"),
        ("XDG_STATE_HOME", "/xdg-state"),
        ("XDG_CACHE_HOME", "/xdg-cache"),
        ("XDG_CONFIG_HOME", "/xdg-config"),
    ]);

    assert!(is_maw_xdg_enabled(&env));
    assert_eq!(maw_runtime_home_dir(&env), PathBuf::from("/xdg-state/maw"));
    assert_eq!(maw_data_dir(&env), PathBuf::from("/xdg-data/maw"));
    assert_eq!(maw_state_dir(&env), PathBuf::from("/xdg-state/maw"));
    assert_eq!(maw_cache_dir(&env), PathBuf::from("/xdg-cache/maw"));
    assert_eq!(maw_config_dir(&env), PathBuf::from("/xdg-config/maw"));
}

#[test]
fn explicit_maw_env_overrides_beat_xdg_mode() {
    let env = env(&[
        ("MAW_XDG", "1"),
        ("MAW_CONFIG_DIR", "/maw-config"),
        ("MAW_DATA_DIR", "/maw-data"),
        ("MAW_STATE_DIR", "/maw-state"),
        ("MAW_CACHE_DIR", "/maw-cache"),
    ]);

    assert_eq!(maw_config_dir(&env), PathBuf::from("/maw-config"));
    assert_eq!(maw_data_dir(&env), PathBuf::from("/maw-data"));
    assert_eq!(maw_state_dir(&env), PathBuf::from("/maw-state"));
    assert_eq!(maw_cache_dir(&env), PathBuf::from("/maw-cache"));
}

#[test]
fn maw_home_keeps_instance_mode_isolated_and_ignores_relative_xdg_bases() {
    let env = env(&[
        ("MAW_HOME", "/maw-home"),
        ("MAW_XDG", "on"),
        ("XDG_DATA_HOME", "relative-data"),
        ("XDG_STATE_HOME", "relative-state"),
        ("XDG_CACHE_HOME", "relative-cache"),
    ]);

    assert_eq!(maw_runtime_home_dir(&env), PathBuf::from("/maw-home"));
    assert_eq!(maw_config_dir(&env), PathBuf::from("/maw-home/config"));
    assert_eq!(maw_data_dir(&env), PathBuf::from("/maw-home"));
    assert_eq!(maw_state_dir(&env), PathBuf::from("/maw-home"));
    assert_eq!(maw_cache_dir(&env), PathBuf::from("/maw-home"));
}

#[test]
fn relative_xdg_env_vars_are_ignored_when_maw_home_is_absent() {
    let env = env(&[
        ("MAW_XDG", "true"),
        ("XDG_DATA_HOME", "relative-data"),
        ("XDG_STATE_HOME", "relative-state"),
        ("XDG_CACHE_HOME", "relative-cache"),
        ("XDG_CONFIG_HOME", "relative-config"),
    ]);

    assert_eq!(
        maw_data_dir(&env),
        home().join(".local").join("share").join("maw")
    );
    assert_eq!(
        maw_state_dir(&env),
        home().join(".local").join("state").join("maw")
    );
    assert_eq!(maw_cache_dir(&env), home().join(".cache").join("maw"));
    assert_eq!(maw_config_dir(&env), home().join(".config").join("maw"));
}
