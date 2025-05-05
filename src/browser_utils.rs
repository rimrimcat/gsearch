use chromiumoxide::{
    Handler,
    browser::Browser,
    cdp::browser_protocol::{
        network::SetUserAgentOverrideParams, page::AddScriptToEvaluateOnNewDocumentParams,
    },
    error::Result,
    page::Page,
};
use fake_user_agent::get_chrome_rua;
use futures::StreamExt;
use scraper::Selector;
use serde::Deserialize;
use std::{error::Error, ffi::OsStr, fs::read_to_string, path::PathBuf, sync::Arc, time::Instant};
use tokio::spawn;

#[derive(Debug, Deserialize)]
pub struct DevToolsInfo {
    #[serde(rename = "webSocketDebuggerUrl")]
    pub websocket_debugger_url: String,
}

#[derive(Clone)]
pub struct PageWrapper {
    pub page: Page,
}

impl PageWrapper {
    pub fn new(page: Page) -> Self {
        Self { page }
    }

    pub async fn close(self) -> Result<()> {
        self.page.close().await?;
        Ok(())
    }
}

pub struct BrowserWrapper {
    pub browser: Arc<Browser>,
}
impl BrowserWrapper {
    pub async fn new(browser: Arc<Browser>, mut handler: Handler) -> Self {
        let handle = tokio::task::spawn(async move {
            loop {
                let _ = handler.next().await.unwrap();
            }
        });

        Self { browser }
    }
}

static SCRIPTS: &[&str] = &[
    "utils.js",
    "chrome_app.js",
    "chrome_runtime.js",
    "iframe_content_window.js",
    "media_codecs.js",
    "navigator_language.js",
    "navigator_permissions.js",
    "navigator_plugins.js",
    "webgl_vendor_override.js",
    "window_outerdimensions.js",
    "hairline_fix.js",
];

pub fn has_element(element: &scraper::element_ref::ElementRef, selector: &str) -> bool {
    element
        .select(&Selector::parse(selector).unwrap())
        .next()
        .is_some()
}

pub fn select_element_text(element: &scraper::element_ref::ElementRef, selector: &str) -> String {
    match element.select(&Selector::parse(selector).unwrap()).next() {
        Some(sel) => sel.text().collect::<Vec<_>>().join(""),
        None => "".to_string(),
    }
}

pub fn select_element_attr(
    element: &scraper::element_ref::ElementRef,
    selector: &str,
    attr: &str,
) -> String {
    match element.select(&Selector::parse(selector).unwrap()).next() {
        Some(sel) => sel.value().attr(attr).unwrap().to_string(),
        None => "".to_string(),
    }
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

pub async fn connect_to_browser(port: u16) -> Result<BrowserWrapper, Box<dyn Error + Send + Sync>> {
    #[cfg(debug_assertions)]
    println!("Connecting to browser...");
    #[cfg(debug_assertions)]
    let start = Instant::now();

    let tuple = Browser::connect(format!("http://localhost:{}", port)).await?;

    let bwrapper = BrowserWrapper::new(Arc::new(tuple.0), tuple.1).await;

    #[cfg(debug_assertions)]
    {
        println!("{}ms: Connected to browser", start.elapsed().as_millis());
        println!(
            "Browser websocket url: {}",
            bwrapper.browser.websocket_address()
        );
    }

    Ok(bwrapper)
}

// taken from outdated package chromiumoxide_stealth
pub async fn inject_stealth(
    page: &Page,
    evasions_scripts_path: Option<PathBuf>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    match evasions_scripts_path {
        Some(evasions_scripts_path) => {
            if evasions_scripts_path.is_dir() {
                #[cfg(debug_assertions)]
                println!("Injecting stealth...");

                for script in SCRIPTS.iter() {
                    page.execute(AddScriptToEvaluateOnNewDocumentParams {
                        source: read_to_string(evasions_scripts_path.join(script)).unwrap(),
                        include_command_line_api: None,
                        world_name: None,
                        run_immediately: true.into(),
                    })
                    .await?;
                }
            } else {
                println!("evasions path specified but not a directory!");
            }
        }
        None => {
            #[cfg(debug_assertions)]
            println!("Not injecting stealth...");
        }
    };

    Ok(())
}

pub async fn make_new_tab(
    browser: &Browser,
    evasions_scripts_path: Option<PathBuf>,
) -> Result<PageWrapper, Box<dyn Error + Send + Sync>> {
    #[cfg(debug_assertions)]
    println!("Creating new page...");
    #[cfg(debug_assertions)]
    let start = Instant::now();

    let page = browser.new_page("chrome://version/").await?;
    #[cfg(debug_assertions)]
    println!("{}ms: Page created", start.elapsed().as_millis());

    inject_stealth(&page, evasions_scripts_path).await?;

    #[cfg(debug_assertions)]
    println!("Tab created in {}ms", start.elapsed().as_millis());

    Ok(PageWrapper::new(page))
}

pub async fn make_or_take_nth_tab(
    browser: &Browser,
    n: u16,
    evasions_scripts_path: Option<PathBuf>,
) -> Result<PageWrapper, Box<dyn Error + Send + Sync>> {
    #[cfg(debug_assertions)]
    println!("Will make or take tab {}...", n);
    #[cfg(debug_assertions)]
    let start = Instant::now();

    let pages = browser.pages().await?;
    let page_len = pages.len();

    let page;

    if page_len > n as usize {
        page = pages[n as usize].clone();

        #[cfg(debug_assertions)]
        println!(
            "{}ms: Cloned and taken tab {}",
            start.elapsed().as_millis(),
            n
        );
    } else {
        page = browser.new_page("chrome://version/").await?;

        inject_stealth(&page, evasions_scripts_path).await?;

        #[cfg(debug_assertions)]
        println!("{}ms: Created new tab", start.elapsed().as_millis());
    }

    #[cfg(debug_assertions)]
    println!(
        "{}ms: make_or_take_nth_tab done",
        start.elapsed().as_millis()
    );

    Ok(PageWrapper::new(page))
}
