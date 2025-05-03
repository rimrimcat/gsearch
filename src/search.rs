use chromiumoxide::{error::Result, page::Page};
use futures::future::join_all;
use scraper::{Html, Selector};
use std::collections::VecDeque;
use std::error::Error;
use std::ffi::OsStr;
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;
use std::time::Instant;
use sysinfo::System;
use tokio::sync::{Mutex, Notify};
use tokio::{spawn, task::JoinHandle};

use crate::browser_utils::{
    connect_to_browser, make_new_tab, select_element_attr, select_element_text,
};

#[derive(Debug, Clone)]
pub enum Engines {
    Google,
}

#[derive(Debug)]
pub struct SearchTask {
    pub engine: Engines,
    pub query: String,
    pub page: u32,
    pub max_results: u32,
}

#[derive(Debug)]
pub struct QueuedSearchTask {
    pub task: SearchTask,
    pub id: u32,
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub title: String,
    pub link: String,
    pub cite: String,
    pub image: String,
    pub description: String,
    pub updated: String,
    pub page: u32,
}

#[derive(Clone)]
pub struct SearchTaskQueue {
    page: Page,
    queue: Arc<Mutex<VecDeque<QueuedSearchTask>>>,
    proc_notify: Arc<Notify>,
    result: Arc<RwLock<Vec<SearchResult>>>,
    result_notify: Arc<Notify>,
    is_running: Arc<AtomicBool>,
    last_finished_task_id: Arc<AtomicU32>,
}

impl SearchTaskQueue {
    pub fn new(page: Page) -> Self {
        Self {
            page,
            queue: Arc::new(Mutex::new(VecDeque::new())),
            proc_notify: Arc::new(Notify::new()),
            result: Arc::new(RwLock::new(Vec::new())),
            result_notify: Arc::new(Notify::new()),
            is_running: Arc::new(AtomicBool::new(false)),
            last_finished_task_id: Arc::new(AtomicU32::new(1000)),
        }
    }

    pub async fn add_task(&self, task: SearchTask) -> u32 {
        let mut id = 0;
        {
            let mut queue = self.queue.lock().await;
            if queue.is_empty() {
                queue.push_back(QueuedSearchTask { task, id });
            } else {
                id = (queue.back().unwrap().id + 1) % 100;
                queue.push_back(QueuedSearchTask { task, id });
            }
        }

        #[cfg(debug_assertions)]
        println!("Task added to queue");

        self.start_processor();
        id
    }

    pub fn get_result(&self) -> Vec<SearchResult> {
        self.result.read().unwrap().clone()
    }

    pub async fn wait_result_update(&self) {
        self.result_notify.notified().await;
    }

    pub fn get_last_finished_task_id(&self) -> u32 {
        self.last_finished_task_id.load(Ordering::SeqCst)
    }

    pub fn start_processor(&self) {
        if self
            .is_running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            #[cfg(debug_assertions)]
            println!("Starting processor...");

            let processor = self.clone();
            tokio::spawn(async move {
                processor.run().await;
            });
        } else {
            self.proc_notify.notify_waiters();
        }
    }

    async fn run(self) {
        loop {
            let maybe_task = {
                let mut queue = self.queue.lock().await;
                queue.pop_front()
            };

            if let Some(qtask) = maybe_task {
                #[cfg(debug_assertions)]
                println!("Starting search: {}", qtask.task.query);
                #[cfg(debug_assertions)]
                let start = Instant::now();

                let __last_task_id = qtask.id;

                let new_result = google_search(
                    &self.page,
                    qtask.task.query,
                    qtask.task.page,
                    qtask.task.max_results,
                )
                .await;

                {
                    let mut curr_result = self.result.write().unwrap();
                    *curr_result = new_result;
                }

                self.last_finished_task_id
                    .store(__last_task_id, Ordering::SeqCst);

                self.result_notify.notify_waiters();

                #[cfg(debug_assertions)]
                println!("Search finished in {}ms", start.elapsed().as_millis());

                let mut queue = self.queue.lock().await;
                if queue.len() > 1 {
                    // After processing, keep only the last task if there are any
                    let last = queue.pop_back().unwrap();
                    queue.clear();
                    queue.push_back(last);
                } else if queue.is_empty() {
                    #[cfg(debug_assertions)]
                    println!("Queue empty, processor exiting...");

                    self.is_running.store(false, Ordering::SeqCst);
                    return;
                }
            }
        }
    }

    pub async fn stop(&self) {
        {
            let mut queue = self.queue.lock().await;
            queue.clear();
        }

        if self.is_running.load(Ordering::SeqCst) {
            self.is_running.store(false, Ordering::SeqCst);
            self.result_notify.notified().await;
        }
        let _ = self.page.clone().close().await;
    }
}

pub fn is_captcha(fragment: &Html) -> bool {
    // div
    // id="recaptcha"
    // class="g-recaptcha"

    if fragment
        .select(&Selector::parse("div#recaptcha").unwrap())
        .next()
        .is_some()
    {
        return true;
    }
    false
}

async fn __google_search(
    page: &Page,
    query: &str,
    page_num: u32,
    max_results: u32, // must be within 1 and 10
) -> Result<Vec<SearchResult>, Box<dyn Error + Send + Sync>> {
    let start_page = match page_num <= 1 {
        true => "".to_string(),
        false => format!("&start={}", ((page_num - 1) * 10).to_string()),
    };
    let page_num = match page_num <= 1 {
        true => 1,
        false => page_num,
    };

    let query_link = format!(
        "https://www.google.com/search?udm=14&dpr=1&q={}{}",
        query, start_page
    );

    page.goto(&query_link).await?;

    let html_str = page.wait_for_navigation().await?.content().await?;

    let html = Html::parse_document(&html_str);

    if is_captcha(&html) {
        #[cfg(debug_assertions)]
        println!("Captcha detected!");

        // let cite = match page.url().await {
        //     Ok(url) => url.unwrap(),
        //     Err(_) => "".to_string(),
        // };

        return Ok(vec![SearchResult {
            title: "Captcha".into(),
            link: query_link.into(),
            cite: "".into(),
            image: "".into(),
            description: "Cannot access due to captcha".into(),
            updated: "".into(),
            page: page_num,
        }]);
    }

    let main_div = html
        .select(&Selector::parse("div#rso").unwrap())
        .next()
        .unwrap();

    let div_selector = Selector::parse("div.MjjYud").unwrap();

    let mut search_results = Vec::new();

    for element in main_div.select(&div_selector).take(max_results as usize) {
        let title = select_element_text(&element, "h3");
        let link = select_element_attr(&element, "a", "href");
        let cite = select_element_text(&element, "cite");

        let image_element = element
            .select(&Selector::parse("img.XNo5Ab").unwrap())
            .next();
        let image = match image_element {
            Some(_) => select_element_attr(&element, "img.XNo5Ab", "src"),
            None => "".to_string(),
        };

        let updated_element = element
            .select(&Selector::parse("span.YrbPuc").unwrap())
            .next();
        let div_to_span_element = element
            .select(&Selector::parse("div.VwiC3b > span").unwrap())
            .next();

        let description = match updated_element {
            Some(_) => select_element_text(&element, "span.YrbPuc + span"),
            None => match div_to_span_element {
                Some(_) => div_to_span_element
                    .unwrap()
                    .text()
                    .collect::<Vec<_>>()
                    .join(""),
                None => select_element_text(&element, "div.VwiC3b"),
            },
        };

        let updated = match updated_element {
            Some(_) => select_element_text(&element, "span.YrbPuc"),
            None => "".to_string(),
        };

        let result = SearchResult {
            title,
            link,
            cite,
            image,
            description,
            updated,
            page: page_num,
        };

        search_results.push(result);
    }

    Ok(search_results)
}

pub async fn google_search(
    page: &Page,
    query: String,
    page_num: u32,
    max_results: u32,
) -> Vec<SearchResult> {
    let result = __google_search(&page, &query, page_num, max_results).await;

    let ret = match result {
        Ok(_) => result,
        Err(_) => Ok(Vec::new()),
    };

    ret.unwrap()
}

pub fn print_search_results(result: &Vec<SearchResult>) {
    for (i, res) in result.iter().enumerate() {
        println!("Result #{}", (res.page as usize - 1) * 10 + i + 1);
        println!("Title: {}", res.title);
        println!("Link: {}", res.link);
        println!("Cite: {}", res.cite);
        println!("Description: {}", res.description);
        println!("Updated: {}", res.updated);
        println!("----------");
    }
}

pub async fn browser_check() -> Result<(), Box<dyn Error + Send + Sync>> {
    let mut system = System::new_all();
    system.refresh_all();

    // Try matching process names like "chromium", "chromium-browser", etc.
    let target_names = ["chromium", "chromium-browser"];

    for name in target_names.iter() {
        if system.processes_by_name(OsStr::new(name)).next().is_some() {
            #[cfg(debug_assertions)]
            println!("Chromium process found.");
            return Ok(());
        }
    }

    #[cfg(debug_assertions)]
    println!("Chromium process not found.");

    Ok(())
}
