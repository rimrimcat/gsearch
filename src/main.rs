use std::error::Error;

mod browser_utils;
use browser_utils::connect_to_browser;
use browser_utils::make_new_tab;
use chromiumoxide::Browser;
use chromiumoxide::BrowserConfig;
use futures::StreamExt;
use search::Engines;
use search::SearchTask;
use search::SearchTaskQueue;

mod search;

async fn main_tokio_wiki() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // tracing_subscriber::fmt::init();

    let (browser, mut handler) =
        Browser::launch(BrowserConfig::builder().with_head().build()?).await?;

    let handle = tokio::task::spawn(async move {
        loop {
            let _ = handler.next().await.unwrap();
        }
    });

    let page = browser.new_page("https://en.wikipedia.org").await?;

    page.find_element("input#searchInput")
        .await?
        .click()
        .await?
        .type_str("Rust programming language")
        .await?
        .press_key("Enter")
        .await?;

    let _html = page.wait_for_navigation().await?.content().await?;

    handle.await?;
    Ok(())
}

pub async fn new_test_search() -> Result<(), Box<dyn Error + Send + Sync>> {
    let use_stealth = true;
    let engine = match use_stealth {
        true => Engines::GoogleAlt,
        false => Engines::Google,
    };
    let evasions_scripts_path = match use_stealth {
        true => Some("src/evasions".into()),
        false => None,
    };

    let bwrapper = connect_to_browser(8928).await?;

    let pagew = make_new_tab(&bwrapper.browser, evasions_scripts_path).await?;

    let task_queue = SearchTaskQueue::new(pagew.page.clone());

    task_queue
        .add_task(SearchTask {
            engine: engine.clone(),
            query: "rust".into(),
            args: None,
        })
        .await;

    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

    task_queue
        .add_task(SearchTask {
            engine: engine.clone(),
            query: "python".into(),
            args: None,
        })
        .await;

    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

    task_queue
        .add_task(SearchTask {
            engine: engine.clone(),
            query: "golang".into(),
            args: None,
        })
        .await;

    tokio::time::sleep(std::time::Duration::from_millis(2000)).await;

    task_queue
        .add_task(SearchTask {
            engine: engine.clone(),
            query: "typescript".into(),
            args: None,
        })
        .await;

    task_queue.wait_result_update().await;
    let final_result = task_queue.get_result();
    println!("Final result len: {}", final_result.len());

    tokio::time::sleep(std::time::Duration::from_millis(3000)).await;

    task_queue
        .add_task(SearchTask {
            engine: engine.clone(),
            query: "sveltekit".into(),
            args: None,
        })
        .await;

    task_queue.wait_result_update().await;

    let final_result = task_queue.get_result();
    println!("Task 5 result description: {}", final_result[0].description);

    tokio::time::sleep(std::time::Duration::from_millis(3000)).await;
    // task_queue.stop().await;

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let _ = new_test_search().await;

    Ok(())
}
