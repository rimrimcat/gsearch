use abi_stable::std_types::{ROption, RString, RVec};
use anyrun_plugin::*;
use headless_chrome::Browser;
use serde::Deserialize;
use std::{fs, process::Command};

mod browser_utils;
use browser_utils::{connect_to_browser, make_new_tab};

mod search;
use search::google_search_async;

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
    browser: Browser,
}

#[init]
#[tokio::main]
async fn init(config_dir: RString) -> State {
    let config = match fs::read_to_string(format!("{}/gsearch.ron", config_dir)) {
        Ok(content) => ron::from_str(&content).unwrap_or_default(),
        Err(_) => Config::default(),
    };

    let port = config.port;

    State {
        config,
        browser: (connect_to_browser(port)).await.unwrap(),
    }
}

#[handler]
fn handler(selection: Match, _state: &State) -> HandleResult {
    if let Err(why) = Command::new("sh")
        .arg("-c")
        .arg(format!("xdg-open \"{}\"", selection.description.unwrap()))
        .spawn()
    {
        println!("Failed to perform websearch: {}", why);
    }

    HandleResult::Close
}

#[get_matches]
#[tokio::main]
async fn get_matches(input: RString, state: &State) -> RVec<Match> {
    if !input.starts_with(&state.config.prefix) {
        return RVec::new();
    }

    let input = input.replace(&state.config.prefix, "");
    if input.is_empty() {
        return RVec::new();
    }

    let tabw = make_new_tab(&state.browser).unwrap();
    let results = google_search_async(tabw, input, 1, state.config.max_results).await;

    results
        .iter()
        .enumerate()
        .map(|(i, res)| Match {
            title: format!("{} ({})", res.description.clone(), res.title.clone()).into(),
            description: ROption::RSome(res.link.clone().into()),
            icon: ROption::RNone,
            id: ROption::RSome(i as u64),
            use_pango: false,
        })
        .collect()
}

#[info]
fn info() -> PluginInfo {
    PluginInfo {
        name: "GSearch".into(),
        icon: "help-about".into(),
    }
}
