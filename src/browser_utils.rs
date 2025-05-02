use fake_user_agent::get_chrome_rua;
use headless_chrome::{Browser, LaunchOptionsBuilder, Tab};
use scraper::Selector;
use serde::Deserialize;
use std::{collections::HashMap, error::Error, ffi::OsStr, sync::Arc, time::Instant};
use tokio::spawn;

#[derive(Debug, Deserialize)]
pub struct DevToolsInfo {
    #[serde(rename = "webSocketDebuggerUrl")]
    pub websocket_debugger_url: String,
}

#[derive(Clone)]
pub struct TabWrapper {
    pub tab: Arc<Tab>,
}

impl TabWrapper {
    pub fn new(tab: Arc<Tab>) -> Self {
        Self { tab }
    }
}

impl Drop for TabWrapper {
    fn drop(&mut self) {
        let _ = self.tab.close_target();
    }
}

pub fn select_element_text(element: &scraper::element_ref::ElementRef, selector: &str) -> String {
    element
        .select(&Selector::parse(selector).unwrap())
        .next()
        .unwrap()
        .text()
        .collect::<Vec<_>>()
        .join("")
}

pub fn select_element_attr(
    element: &scraper::element_ref::ElementRef,
    selector: &str,
    attr: &str,
) -> String {
    element
        .select(&Selector::parse(selector).unwrap())
        .next()
        .unwrap()
        .value()
        .attr(attr)
        .unwrap()
        .to_string()
}

pub fn check_chromium() -> bool {
    let mut system = sysinfo::System::new_all();
    system.refresh_all();

    let target_names = ["chromium", "chromium-browser"];

    for name in target_names.iter() {
        if system.processes_by_name(OsStr::new(name)).next().is_some() {
            return true;
        }
    }

    false
}

pub fn spawn_browser() -> Result<Browser, Box<dyn Error + Send + Sync>> {
    let default_options = LaunchOptionsBuilder::default()
        .ignore_default_args(vec![OsStr::new("--enable-automation")])
        .build()?;

    Ok(Browser::new(default_options)?)
}

pub async fn connect_to_browser(port: u16) -> Result<Browser, Box<dyn Error + Send + Sync>> {
    let fut = spawn(reqwest::get(format!(
        "http://localhost:{}/json/version",
        port
    )))
    .await?;

    if fut.is_err() {
        return spawn_browser();
    }

    let fut_json: Result<DevToolsInfo, reqwest::Error> = match fut {
        Ok(_) => fut.unwrap().json().await,
        Err(_) => return Err("Failed to connect to browser".into()),
    };

    let res = match fut_json {
        Ok(_) => fut_json.unwrap() as DevToolsInfo,
        Err(_) => return Err("Failed to connect to browser".into()),
    };

    let browser = Browser::connect(res.websocket_debugger_url)?;

    Ok(browser)
}

pub fn make_new_tab(browser: &Browser) -> Result<TabWrapper, Box<dyn Error + Send + Sync>> {
    #[cfg(debug_assertions)]
    println!("Creating new tab...");
    #[cfg(debug_assertions)]
    let start = Instant::now();

    let custom_user_agent = get_chrome_rua();

    let mut headers = HashMap::new();
    headers.insert("User-Agent", custom_user_agent);

    let tab = browser.new_tab()?;
    tab.set_extra_http_headers(headers)?;
    tab.enable_stealth_mode()?;

    #[cfg(debug_assertions)]
    println!("Tab created in {}ms", start.elapsed().as_millis());

    Ok(TabWrapper::new(tab))
}
