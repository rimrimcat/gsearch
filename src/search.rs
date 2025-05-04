use chromiumoxide::{error::Result, page::Page};
use scraper::{Html, Selector};
use std::error::Error;
use std::ffi::OsStr;
use std::time::Instant;
use sysinfo::System;

use crate::browser_utils::{select_element_attr, select_element_text};

#[derive(Debug, Clone)]
pub enum Engines {
    Google,
    GoogleAlt,
}

impl Engines {
    pub fn url_template(&self) -> String {
        match self {
            Engines::Google => "Google".to_string(),
            Engines::GoogleAlt => "GoogleStealth".to_string(),
        }
    }

    pub async fn search(
        &self,
        page: &Page,
        query: String,
        args: Option<SearchArguments>,
    ) -> Vec<SearchResult> {
        let result = match self {
            Engines::Google => search_google(page, query, args.unwrap_or_default()).await,
            Engines::GoogleAlt => search_google_alt(page, query, args.unwrap_or_default()).await,
        };

        match result {
            Ok(results) => results,
            Err(_) => Vec::new(),
        }
    }
}

#[derive(Debug)]
pub struct SearchTask {
    pub engine: Engines,
    pub query: String,
    pub args: Option<SearchArguments>,
}

#[derive(Debug)]
pub struct SearchArguments {
    pub page: u32,
    pub max_results: u32,
}

impl SearchArguments {
    pub fn new(page: u32, max_results: u32) -> Self {
        Self { page, max_results }
    }

    pub fn default() -> Self {
        Self::new(1, 10)
    }
}

impl Default for SearchArguments {
    fn default() -> Self {
        Self::new(1, 10)
    }
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

fn google_alt_extract_url(input: String) -> Option<String> {
    let q_start = input.find("q=")?;
    let start_index = q_start + 2;

    let remaining = &input[start_index..];
    let end_index = remaining.find('&')?;

    Some(remaining[..end_index].to_string())
}

async fn search_google(
    page: &Page,
    query: String,
    args: SearchArguments,
) -> Result<Vec<SearchResult>, Box<dyn Error + Send + Sync>> {
    #[cfg(debug_assertions)]
    println!("Starting google search for {}", query);
    #[cfg(debug_assertions)]
    let start = Instant::now();

    let page_num = args.page;
    let max_results = args.max_results;

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
    #[cfg(debug_assertions)]
    println!("{}ms: Page loaded", start.elapsed().as_millis());

    let html_str = page.wait_for_navigation().await?.content().await?;

    let html = Html::parse_document(&html_str);
    #[cfg(debug_assertions)]
    println!("{}ms: HTML parsed", start.elapsed().as_millis());

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
        #[cfg(debug_assertions)]
        println!(
            "{}ms: Extracting result {}",
            start.elapsed().as_millis(),
            search_results.len()
        );

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

    #[cfg(debug_assertions)]
    println!("{}ms: Search finished", start.elapsed().as_millis());

    Ok(search_results)
}

async fn search_google_alt(
    page: &Page,
    query: String,
    args: SearchArguments,
) -> Result<Vec<SearchResult>, Box<dyn Error + Send + Sync>> {
    #[cfg(debug_assertions)]
    println!("Starting google alt search for {}", query);
    #[cfg(debug_assertions)]
    let start = Instant::now();

    // IDK WHY, BUT RUNNING THIS MAKES YOU ACCESS AN ALTERNATIVE GOOGLE SITE?
    page.set_user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/535.1 (KHTML, like Gecko) Chrome/13.0.782.41 Safari/535.1").await?;

    let page_num = args.page;
    let max_results = args.max_results;

    let start_page = match page_num <= 1 {
        true => "".to_string(),
        false => format!("&start={}", ((page_num - 1) * 10).to_string()),
    };
    let page_num = match page_num <= 1 {
        true => 1,
        false => page_num,
    };

    let query_link = format!(
        "https://www.google.com/search?dpr=1&udm=14&q={}{}",
        query, start_page
    );

    page.goto(&query_link).await?;
    #[cfg(debug_assertions)]
    println!("{}ms: Page loaded", start.elapsed().as_millis());

    let html_str = page.wait_for_navigation().await?.content().await?;

    let html = Html::parse_document(&html_str);
    #[cfg(debug_assertions)]
    println!("{}ms: HTML parsed", start.elapsed().as_millis());

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

    let main_body = html
        .select(&Selector::parse("body").unwrap())
        .next()
        .unwrap();

    let div_selector = Selector::parse("div.ezO2md").unwrap();

    let mut search_results = Vec::new();

    for element in main_body.select(&div_selector).take(max_results as usize) {
        #[cfg(debug_assertions)]
        println!(
            "{}ms: Extracting result {}",
            start.elapsed().as_millis(),
            search_results.len()
        );

        let title = select_element_text(&element, "span.CVA68e");
        let link = match google_alt_extract_url(select_element_attr(&element, "a.fuLhoc", "href")) {
            Some(_url) => _url,
            None => "".to_string(),
        };
        let cite = select_element_text(&element, "span.qXLe6d.dXDvrc > span.fYyStc");
        let description = select_element_text(&element, "span.qXLe6d.FrIlee > span.fYyStc");

        let result = SearchResult {
            title,
            link,
            cite,
            image: "".into(),
            description,
            updated: "".into(),
            page: page_num,
        };

        search_results.push(result);
    }

    #[cfg(debug_assertions)]
    println!("{}ms: Search finished", start.elapsed().as_millis());

    Ok(search_results)
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
