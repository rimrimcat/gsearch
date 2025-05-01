use std::fs;

use abi_stable::std_types::{RString, RVec};
use anyrun_plugin::*;
use serde::Deserialize;

#[derive(Deserialize)]
struct Config {
    prefix: String,
    port: u16,
    max_results: u32,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            prefix: "@".to_string(),
            port: 8928,
            max_results: 3,
        }
    }
}

struct State {
    config: Config,
}

// #[init]
fn init(config_dir: RString) -> Config {
    match fs::read_to_string(format!("{}/gsearch.ron", config_dir)) {
        Ok(content) => ron::from_str(&content).unwrap_or_default(),
        Err(_) => Config::default(),
    }
}

//#[handler]
fn handler(_match: Match) -> HandleResult {
    HandleResult::Copy(_match.title.into_bytes())
}

// #[get_mathes]
fn get_matches(input: RString, config: &Config) -> RVec<Match> {
    RVec::new()
}

// #[info]
fn info() -> PluginInfo {
    PluginInfo {
        name: "GSearch".into(),
        icon: "help-about".into(),
    }
}
