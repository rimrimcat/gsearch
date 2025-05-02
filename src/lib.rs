use abi_stable::std_types::{ROption, RString, RVec};
use anyrun_plugin::*;
use headless_chrome::Browser;
use serde::Deserialize;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::SeqCst;
use std::thread::sleep;
use std::time::Duration;
use std::{fs, process::Command, sync::Arc};
use tokio::sync::Mutex;
use tokio::sync::{Notify, RwLock};

mod browser_utils;
use browser_utils::TabWrapper;
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
            type_max_delay: 1000,
        }
    }
}

struct State {
    config: Config,
    browser: headless_chrome::Browser,
    tabw: TabWrapper,
    task_queue: SearchTaskQueue,
    last_stored_result: Arc<RwLock<RVec<Match>>>,
    result_notify: Arc<Notify>,
    current_query: Arc<Mutex<String>>,
    has_result: Arc<AtomicBool>,
}

fn _typing() -> RVec<Match> {
    vec![Match {
        title: "Typing...".into(),
        description: ROption::RSome("still typing...".into()),
        icon: ROption::RNone,
        id: ROption::RSome(0 as u64),
        use_pango: false,
    }]
    .into()
}

#[init]
#[tokio::main]
async fn init(config_dir: RString) -> State {
    let config = match fs::read_to_string(format!("{}/gsearch.ron", config_dir)) {
        Ok(content) => ron::from_str(&content).unwrap_or_default(),
        Err(_) => Config::default(),
    };

    let port = config.port;
    let browser = connect_to_browser(port).await.unwrap();
    let tabw = browser_utils::make_or_take_nth_tab(&browser.clone(), 2).unwrap();
    let task_queue = SearchTaskQueue::new(tabw.tab.clone());

    State {
        config,
        browser,
        tabw,
        task_queue,
        last_stored_result: Arc::new(RwLock::new(RVec::new())),
        result_notify: Arc::new(Notify::new()),
        current_query: Arc::new(Mutex::new("".to_string())),
        has_result: Arc::new(AtomicBool::new(false)),
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

    sleep(Duration::from_millis(state.config.type_max_delay as u64));

    if *state.current_query.lock().await != input {
        println!("Still typing, will resturn last stored result");
        // if state.has_result.load(SeqCst) {
        //     println!("Returning last stored result");
        //     return state.last_stored_result.read().await.clone();
        // } else {
        //     println!("Returning typing...");
        //     return _typing();
        // }

        return state.last_stored_result.read().await.clone();
    }

    println!("will start task");
    let instant = std::time::Instant::now();

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
        println!(
            "TASK {}: waiting for result since > last_task_id {}",
            task_id, last_task_id
        );
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

        println!(
            "adding {} results, took {}ms",
            final_results.len(),
            instant.elapsed().as_millis()
        );
        println!(
            "results {:?}",
            results
                .iter()
                .map(|res| { res.description.clone() })
                .collect::<Vec<String>>()
        );

        {
            let mut curr_result = state.last_stored_result.write().await;
            *curr_result = final_results.clone();

            let _ = state
                .has_result
                .compare_exchange(false, true, SeqCst, SeqCst);
        }

        state.result_notify.notify_waiters();

        return final_results;
    } else {
        println!("waiting for result");
        state.result_notify.notified().await;

        println!("returning last stored result");
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
