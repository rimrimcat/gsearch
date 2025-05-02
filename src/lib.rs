use abi_stable::std_types::{ROption, RString, RVec};
use anyrun_plugin::*;
use serde::Deserialize;
use std::thread::sleep;
use std::time::Duration;
use std::{fs, process::Command, sync::Arc};
use tokio::sync::Mutex;
use tokio::sync::{Notify, RwLock};

mod browser_utils;
use browser_utils::connect_to_browser;

mod search;
use search::{Engines, SearchTask, SearchTaskQueue};

#[derive(Deserialize)]
struct Config {
    prefix: String,
    port: u16,
    max_results: u32,
    type_max_delay: u32,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            prefix: "@".to_string(),
            port: 8928,
            max_results: 3,
            type_max_delay: 200,
        }
    }
}

struct State {
    config: Config,
    task_queue: SearchTaskQueue,
    last_stored_result: Arc<RwLock<RVec<Match>>>,
    result_notify: Arc<Notify>,
    current_query: Arc<Mutex<String>>,
}

#[init]
#[tokio::main]
async fn init(config_dir: RString) -> State {
    let config = match fs::read_to_string(format!("{}/gsearch.ron", config_dir)) {
        Ok(content) => ron::from_str(&content).unwrap_or_default(),
        Err(_) => Config::default(),
    };

    let port = config.port;
    let task_queue = SearchTaskQueue::new((connect_to_browser(port)).await.unwrap());

    State {
        config,
        task_queue,
        last_stored_result: Arc::new(RwLock::new(RVec::new())),
        result_notify: Arc::new(Notify::new()),
        current_query: Arc::new(Mutex::new("".to_string())),
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
    if input.is_empty() || input.len() < 2 {
        return RVec::new();
    }

    // check if typing
    *state.current_query.lock().await = input.clone();

    println!(
        "QUERY:{}, sleeping, will wait for {}ms",
        input, state.config.type_max_delay
    );

    sleep(Duration::from_millis(state.config.type_max_delay as u64));

    if *state.current_query.lock().await != input {
        println!("typing, not searching");
        return state.last_stored_result.read().await.clone();
    }

    println!("not typing anymore, ig ill do a search for {}", input);

    let task_id = state
        .task_queue
        .add_task(SearchTask {
            engine: Engines::Google,
            query: input.clone(),
            page: 1,
            max_results: state.config.max_results,
        })
        .await;

    let _ = &state.task_queue.wait_result_update().await;

    let last_task_id = state.task_queue.get_last_finished_task_id();
    if task_id > last_task_id {
        let _ = &state.task_queue.wait_result_update().await;
    }

    let last_task_id = state.task_queue.get_last_finished_task_id();

    if task_id == last_task_id {
        let results = &state.task_queue.get_result();

        let final_results = results
            .iter()
            .enumerate()
            .map(|(i, res)| Match {
                title: format!("{} ({})", res.description.clone(), res.title.clone()).into(),
                description: ROption::RSome(res.link.clone().into()),
                icon: ROption::RNone,
                id: ROption::RSome(i as u64),
                use_pango: false,
            })
            .collect::<RVec<_>>();

        {
            let mut curr_result = state.last_stored_result.write().await;
            *curr_result = final_results.clone();
        }
        state.result_notify.notify_waiters();

        return final_results;
    } else {
        state.result_notify.notified().await;

        return state.last_stored_result.read().await.clone();
    }
}

#[info]
fn info() -> PluginInfo {
    PluginInfo {
        name: "GSearch".into(),
        icon: "help-about".into(),
    }
}
