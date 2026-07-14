#![allow(
    clippy::pedantic,
    clippy::module_name_repetitions,
    clippy::too_many_lines
)]

pub mod gateway;

use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    env, fs,
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

mod access_core;
mod access_read;
mod access_write;
mod bind;
mod command_dispatch;
mod discord_runtime;
mod discord_state_helpers;
mod pair_route;
mod rest_commands;
mod rest_helpers;
#[allow(dead_code)]
mod serve;
mod status;
mod status_emit;
mod tokens;
mod validation;
mod wind_discord_serve;

pub use self::command_dispatch::{
    run_discord_command, run_discord_command_with, run_discord_command_with_pane_relay,
};
pub use self::discord_runtime::{
    DiscordEnv, DiscordHttpResponse, DiscordOutput, DiscordRest, ReqwestDiscordRest, TokenEntry,
};
pub use self::discord_state_helpers::is_numeric_snowflake;

pub trait DiscordPaneRelay: Send + Sync {
    fn relay<'a>(
        &'a self,
        target: &'a str,
        body: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + 'a>>;
}

use self::{
    access_core::*, access_read::*, access_write::*, bind::*, discord_runtime::*,
    discord_state_helpers::*, pair_route::*, rest_commands::*, rest_helpers::*, status::*,
    status_emit::*, tokens::*, validation::*,
};

#[cfg(test)]
mod command_dispatch_tests;
#[cfg(test)]
mod tokens_tests;
