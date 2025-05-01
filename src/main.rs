use futures::future::join_all;
use headless_chrome::Tab;
use scraper::{Html, Selector};
use std::error::Error;
use std::ffi::OsStr;
use std::sync::Arc;
use sysinfo::System;
use tokio::{spawn, task::JoinHandle};

pub mod browser_utils;
use browser_utils::{
    TabWrapper, connect_to_browser, make_new_tab, select_element_attr, select_element_text,
};

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

pub fn google_search(
    tab: &Arc<Tab>,
    query: &str,
    page: u32,
    max_results: u32, // must be within 1 and 10
) -> Result<Vec<SearchResult>, Box<dyn Error>> {
    let start_page = match page <= 1 {
        true => "".to_string(),
        false => format!("&start={}", ((page - 1) * 10).to_string()),
    };

    tab.navigate_to(&format!(
        "https://www.google.com/search?udm=14&dpr=1&q={}{}",
        query, start_page
    ))?
    .wait_until_navigated()?;

    let html = tab.get_content()?;
    let fragment = Html::parse_document(&html);

    let main_div = fragment
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
            page: match page <= 1 {
                true => 1,
                false => page,
            },
        };

        search_results.push(result);
    }

    Ok(search_results)
}

pub async fn google_search_async(
    tabw: TabWrapper,
    query: String,
    page: u32,
    max_results: u32,
) -> Vec<SearchResult> {
    let result = google_search(&tabw.tab, &query, page, max_results);

    let ret = match result {
        Ok(_) => result,
        Err(_) => Ok(Vec::new()),
    };

    ret.unwrap()
}

fn print_search_results(result: &Vec<SearchResult>) {
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

// stuff
async fn test_search() -> Result<(), Box<dyn Error + Send + Sync>> {
    let browser = connect_to_browser(8928).await?;

    let search_queries = vec![
        ("rust", 1, 10),
        ("rusty", 1, 10),
        ("rust programming language", 1, 10),
    ];

    let mut tasks: Vec<JoinHandle<Vec<SearchResult>>> = Vec::new();

    for (query, page, max_results) in search_queries.clone() {
        let browser_ref = browser.clone();

        let task = spawn(async move {
            let tabw = make_new_tab(&browser_ref);
            if tabw.is_err() {
                return Vec::new();
            }

            google_search_async(tabw.unwrap(), query.to_string(), page, max_results).await
        });

        tasks.push(task);
    }

    let results = join_all(tasks).await;

    for (i, task_result) in results.into_iter().enumerate() {
        match task_result {
            Ok(search_results) => {
                println!(
                    "\n===== SEARCH RESULTS FOR: {} =====\n",
                    search_queries[i].0
                );
                print_search_results(&search_results);
            }
            Err(e) => println!("Task {} panicked: {}", i, e),
        }
    }

    Ok(())
}

async fn browser_check() -> Result<(), Box<dyn Error + Send + Sync>> {
    let mut system = System::new_all();
    system.refresh_all();

    // Try matching process names like "chromium", "chromium-browser", etc.
    let target_names = ["chromium", "chromium-browser"];

    for name in target_names.iter() {
        if system.processes_by_name(OsStr::new(name)).next().is_some() {
            println!("Chromium process found.");
            return Ok(());
        }
    }

    println!("Chromium process not found.");

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    test_search().await?;

    Ok(())
}
