// Copyright (c) Meta Platforms, Inc. and affiliates.
//
// This software may be used and distributed according to the terms of the
// GNU General Public License version 2.

use crate::cli::TuiArgs;
use crate::keymap::parse_action;
use crate::keymap::parse_key;
use crate::AppTheme;
use crate::KeyMap;
use crate::STATS_SOCKET_PATH;
use crate::TRACE_FILE_PREFIX;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use xdg;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Config {
    /// Key mappings.
    pub keymap: Option<HashMap<String, String>>,
    /// Parsed keymap.
    #[serde(skip)]
    pub active_keymap: KeyMap,
    /// TUI theme.
    theme: Option<AppTheme>,
    /// App tick rate in milliseconds.
    tick_rate_ms: Option<usize>,
    /// Extra verbose output.
    debug: Option<bool>,
    /// Exclude bpf event tracking.
    exclude_bpf: Option<bool>,
    /// Stats unix socket path.
    stats_socket_path: Option<String>,
    /// Trace file prefix for perfetto traces.
    trace_file_prefix: Option<String>,
    /// Number of ticks for traces.
    trace_ticks: Option<usize>,
    /// Number of worker threads
    worker_threads: Option<u16>,
    /// Number of ticks to warmup before collecting traces.
    trace_tick_warmup: Option<usize>,
}

impl From<TuiArgs> for Config {
    fn from(args: TuiArgs) -> Config {
        Config {
            keymap: None,
            active_keymap: KeyMap::empty(),
            debug: args.debug,
            exclude_bpf: args.exclude_bpf,
            stats_socket_path: args.stats_socket_path,
            theme: None,
            tick_rate_ms: args.tick_rate_ms,
            trace_file_prefix: args.trace_file_prefix,
            trace_tick_warmup: args.trace_tick_warmup,
            trace_ticks: args.trace_ticks,
            worker_threads: args.worker_threads,
        }
    }
}

pub fn get_config_path() -> Result<PathBuf> {
    let xdg_dirs = xdg::BaseDirectories::with_prefix("scxtop")?;
    let config_path = xdg_dirs.get_config_file("scxtop.toml");
    Ok(config_path)
}

impl Config {
    pub fn merge<I: IntoIterator<Item = Self>>(iter: I) -> Self {
        iter.into_iter().fold(Self::empty_config(), Self::or)
    }

    pub fn or(self, rhs: Self) -> Self {
        let active_keymap = if self.keymap.is_some() {
            self.active_keymap
        } else {
            rhs.active_keymap
        };

        Self {
            keymap: self.keymap.or(rhs.keymap),
            active_keymap,
            theme: self.theme.or(rhs.theme),
            tick_rate_ms: self.tick_rate_ms.or(rhs.tick_rate_ms),
            debug: self.debug.or(rhs.debug),
            exclude_bpf: self.exclude_bpf.or(rhs.exclude_bpf),
            stats_socket_path: self.stats_socket_path.or(rhs.stats_socket_path),
            trace_file_prefix: self.trace_file_prefix.or(rhs.trace_file_prefix),
            trace_ticks: self.trace_ticks.or(rhs.trace_ticks),
            worker_threads: self.worker_threads.or(rhs.worker_threads),
            trace_tick_warmup: self.trace_tick_warmup.or(rhs.trace_tick_warmup),
        }
    }

    /// App theme.
    pub fn theme(&self) -> &AppTheme {
        match &self.theme {
            Some(theme) => theme,
            None => &AppTheme::Default,
        }
    }

    /// Set the app theme.
    pub fn set_theme(&mut self, theme: AppTheme) {
        self.theme = Some(theme);
    }

    /// App tick rate in milliseconds.
    pub fn tick_rate_ms(&self) -> usize {
        self.tick_rate_ms.unwrap_or(250)
    }

    /// Set app tick rate in milliseconds.
    pub fn set_tick_rate_ms(&mut self, tick_rate_ms: usize) {
        self.tick_rate_ms = Some(tick_rate_ms);
    }

    /// Extra verbose output.
    pub fn debug(&self) -> bool {
        self.debug.unwrap_or(false)
    }

    /// Exclude bpf event tracking.
    pub fn exclude_bpf(&self) -> bool {
        self.exclude_bpf.unwrap_or(false)
    }

    /// Stats unix socket path.
    pub fn stats_socket_path(&self) -> &str {
        match &self.stats_socket_path {
            Some(stats_socket_path) => stats_socket_path,
            None => STATS_SOCKET_PATH,
        }
    }

    /// Trace file prefix for perfetto traces.
    pub fn trace_file_prefix(&self) -> &str {
        match &self.trace_file_prefix {
            Some(trace_file_prefix) => trace_file_prefix,
            None => TRACE_FILE_PREFIX,
        }
    }

    /// Number of ticks for traces.
    pub fn trace_ticks(&self) -> usize {
        self.trace_ticks.unwrap_or(5)
    }

    /// Number of worker threads
    pub fn worker_threads(&self) -> u16 {
        self.worker_threads.unwrap_or(4)
    }

    /// Number of ticks to warmup before collecting traces.
    pub fn trace_tick_warmup(&self) -> usize {
        self.trace_tick_warmup.unwrap_or(3)
    }

    /// Returns a config with nothing set.
    pub fn empty_config() -> Config {
        Config {
            keymap: None,
            active_keymap: KeyMap::empty(),
            theme: None,
            tick_rate_ms: None,
            debug: None,
            exclude_bpf: None,
            stats_socket_path: None,
            trace_file_prefix: None,
            trace_ticks: None,
            worker_threads: None,
            trace_tick_warmup: None,
        }
    }

    /// Returns the default config.
    pub fn default_config() -> Config {
        let mut config = Config {
            keymap: None,
            active_keymap: KeyMap::default(),
            theme: None,
            tick_rate_ms: None,
            debug: None,
            exclude_bpf: None,
            stats_socket_path: None,
            trace_file_prefix: None,
            trace_ticks: None,
            worker_threads: None,
            trace_tick_warmup: None,
        };
        config.tick_rate_ms = Some(config.tick_rate_ms());
        config.debug = Some(config.debug());
        config.exclude_bpf = Some(config.exclude_bpf());

        config
    }

    /// Loads the config from XDG configuration.
    pub fn load() -> Result<Config> {
        let config_path = get_config_path()?;
        let contents = fs::read_to_string(config_path)?;
        let mut config: Config = toml::from_str(&contents)?;

        if let Some(keymap_config) = &config.keymap {
            let mut keymap = KeyMap::default();
            for (key_str, action_str) in keymap_config {
                let key = parse_key(key_str)?;
                let action = parse_action(action_str)?;
                keymap.insert(key, action);
            }
            config.active_keymap = keymap;
        } else {
            config.active_keymap = KeyMap::default();
        }

        Ok(config)
    }

    /// Saves the current config.
    pub fn save(&mut self) -> Result<()> {
        self.keymap = Some(self.active_keymap.to_hashmap());
        let config_path = get_config_path()?;
        if !config_path.exists() {
            fs::create_dir_all(config_path.parent().map(PathBuf::from).unwrap())?;
        }
        let config_str = toml::to_string(&self)?;
        Ok(fs::write(&config_path, config_str)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_configs() {
        let mut a = Config::empty_config();
        a.theme = Some(AppTheme::MidnightGreen);
        a.tick_rate_ms = None;
        a.debug = Some(true);
        a.exclude_bpf = None;

        let mut b = Config::empty_config();
        b.theme = Some(AppTheme::IAmBlue);
        b.tick_rate_ms = Some(114);
        b.debug = None;
        a.exclude_bpf = None;

        let merged = Config::merge([a, b]);

        assert_eq!(merged.theme(), &AppTheme::MidnightGreen);
        assert_eq!(merged.tick_rate_ms(), 114);
        assert_eq!(merged.debug(), true);
        assert_eq!(merged.exclude_bpf(), false);
    }
}
