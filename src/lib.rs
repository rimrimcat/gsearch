use abi_stable::std_types::{ROption, RString, RVec};
use anyrun_plugin::*;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use std::thread::sleep;
use std::time::Duration;
use std::{fs, process::Command, sync::Arc};
use tokio::sync::{Mutex, Notify, RwLock};

mod browser_utils;
use browser_utils::{BrowserWrapper, PageWrapper, connect_to_browser, make_or_take_nth_tab};

mod search;
use search::{Engines, SearchArguments, SearchTask, SearchTaskQueue};

#[derive(Deserialize)]
struct Config {
    prefix: String,
    port: u16,
    max_results: u32,
    type_max_delay: u32,
    evasion_scripts_path: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            prefix: "@".to_string(),
            port: 8928,
            max_results: 3,
            type_max_delay: 400,
            evasion_scripts_path: Some(PathBuf::from("~/.config/anyrun/evasions")),
        }
    }
}

struct State {
    config: Config,
    browser_wrapper: BrowserWrapper,
    page_wrapper: PageWrapper,
    task_queue: SearchTaskQueue,
    last_stored_result: Arc<RwLock<RVec<Match>>>,
    result_notify: Arc<Notify>,
    current_query: Arc<Mutex<String>>,
    has_result: Arc<AtomicBool>,
    use_stealth: bool,
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

    let use_stealth = match &config.evasion_scripts_path {
        Some(path) => path.is_dir(),
        None => false,
    };

    #[cfg(debug_assertions)]
    match use_stealth {
        true => println!("Using stealth mode"),
        false => println!("Not using stealth mode"),
    };

    let port = config.port;
    let browser_wrapper = connect_to_browser(port).await.unwrap();
    let page_wrapper = make_or_take_nth_tab(
        &browser_wrapper.browser.clone(),
        2,
        if use_stealth {
            config.evasion_scripts_path.clone()
        } else {
            None
        },
    )
    .await
    .unwrap();
    let task_queue = SearchTaskQueue::new(page_wrapper.page.clone());

    State {
        config,
        browser_wrapper,
        page_wrapper,
        task_queue,
        last_stored_result: Arc::new(RwLock::new(RVec::new())),
        result_notify: Arc::new(Notify::new()),
        current_query: Arc::new(Mutex::new("".to_string())),
        has_result: Arc::new(AtomicBool::new(false)),
        use_stealth,
    }
}

#[handler]
fn handler(selection: Match, _state: &State) -> HandleResult {
    if selection.description == ROption::RNone {
        return HandleResult::Close;
    }

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
        return _typing();
    }

    // check if typing
    *state.current_query.lock().await = input.clone();

    #[cfg(debug_assertions)]
    println!("will sleep for {}ms", state.config.type_max_delay);

    sleep(Duration::from_millis(state.config.type_max_delay as u64));

    if *state.current_query.lock().await != input {
        #[cfg(debug_assertions)]
        println!("Still typing, will resturn last stored result");

        if state.has_result.load(SeqCst) {
            return state.last_stored_result.read().await.clone();
        } else {
            return _typing();
        }

        // return state.last_stored_result.read().await.clone();
    }

    #[cfg(debug_assertions)]
    println!("will start task");

    let instant = std::time::Instant::now();

    let task_id = state
        .task_queue
        .add_task(SearchTask {
            engine: match state.use_stealth {
                true => Engines::GoogleStealth,
                false => Engines::Google,
            },
            query: input.clone(),
            args: Some(SearchArguments::new(1, state.config.max_results)),
        })
        .await;

    let _ = &state.task_queue.wait_result_update().await;

    let last_task_id = state.task_queue.get_last_finished_task_id();
    if task_id > last_task_id {
        #[cfg(debug_assertions)]
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
                title: format!(
                    "{}\n<span weight=\"bold\">{}</span>",
                    res.description.clone(),
                    res.title.clone()
                )
                .into(),
                description: ROption::RSome(res.link.clone().into()),
                icon: ROption::RNone,
                id: ROption::RSome(i as u64),
                use_pango: true,
            })
            .collect::<RVec<_>>();

        #[cfg(debug_assertions)]
        {
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
        }

        {
            let mut curr_result = state.last_stored_result.write().await;
            *curr_result = final_results.clone();

            let _ = state
                .has_result
                .compare_exchange(false, true, SeqCst, SeqCst);
        }

        state.result_notify.notify_waiters();

        #[cfg(debug_assertions)]
        println!("notified and will return final results");
        return final_results;
    } else {
        #[cfg(debug_assertions)]
        println!("waiting for result");
        state.result_notify.notified().await;

        #[cfg(debug_assertions)]
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
