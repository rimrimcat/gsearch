use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::browser_utils::{BrowserWrapper, PageWrapper, connect_to_browser, make_new_tab};
use crate::search::{Engines, SearchArguments, SearchResult, SearchTask};
use crate::search_queue::SearchTaskQueue;

#[derive(Debug)]
pub enum Error {
    BrowserConnection(String),
    ChannelClosed,
    ResponseChannelClosed,
    JoinError(String),
}
pub struct BrowserState {
    browser_wrapper: BrowserWrapper,
    page_wrapper: PageWrapper,
    task_queue: SearchTaskQueue,
    last_activity: Instant,
}
pub enum BrowserCommand {
    Search(String, oneshot::Sender<Vec<SearchResult>>),
    GetPage(oneshot::Sender<PageWrapper>),
    KeepAlive,
    Shutdown,
}

pub struct BrowserThread {
    command_tx: Sender<BrowserCommand>,
    handle: JoinHandle<()>,
}

impl BrowserThread {
    pub async fn new(port: u16, evasion_scripts_path: Option<PathBuf>) -> Result<Self, Error> {
        let (command_tx, command_rx) = mpsc::channel(32);

        let browser_wrapper = connect_to_browser(port).await.unwrap();
        let page_wrapper = make_new_tab(&browser_wrapper.browser, evasion_scripts_path.clone())
            .await
            .unwrap();
        let task_queue = SearchTaskQueue::new(page_wrapper.page.clone());

        let browser_state = BrowserState {
            browser_wrapper,
            page_wrapper,
            task_queue,
            last_activity: Instant::now(),
        };

        let handle = tokio::spawn(Self::run_browser_thread(command_rx, browser_state));

        Ok(Self { command_tx, handle })
    }

    pub async fn run_browser_thread(
        mut command_rx: Receiver<BrowserCommand>,
        mut state: BrowserState,
    ) {
        const TIMEOUT_DURATION: Duration = Duration::from_secs(30);

        loop {
            if state.last_activity.elapsed() > TIMEOUT_DURATION {
                #[cfg(debug_assertions)]
                println!("Browser thread shutting down due to inactivity");
                break;
            }

            match tokio::time::timeout(Duration::from_secs(5), command_rx.recv()).await {
                Ok(Some(command)) => {
                    state.last_activity = Instant::now();

                    match command {
                        BrowserCommand::Search(query, response_tx) => {
                            let results = Self::perform_search(&mut state, query).await;
                            let _ = response_tx.send(results); // Ignore send errors
                        }
                        BrowserCommand::GetPage(response_tx) => {
                            let _ = response_tx.send(state.page_wrapper.clone());
                        }
                        BrowserCommand::KeepAlive => {}
                        BrowserCommand::Shutdown => {
                            #[cfg(debug_assertions)]
                            println!("Browser thread received shutdown command");
                            break;
                        }
                    }
                }
                Ok(None) => {
                    #[cfg(debug_assertions)]
                    println!("Browser thread channel closed");
                    break;
                }
                Err(_) => {}
            }
        }

        state.task_queue.stop().await;
        #[cfg(debug_assertions)]
        println!("Browser thread terminated");
    }

    pub async fn perform_search(state: &mut BrowserState, query: String) -> Vec<SearchResult> {
        let task_id = state
            .task_queue
            .add_task(SearchTask {
                engine: Engines::GoogleAlt,
                query: query.clone(),
                args: Some(SearchArguments::new(1, 3)),
            })
            .await;

        let _ = state.task_queue.wait_result_update().await;

        let last_task_id = state.task_queue.get_last_finished_task_id();
        if task_id > last_task_id {
            let _ = state.task_queue.wait_result_update().await;
        }

        if task_id == state.task_queue.get_last_finished_task_id() {
            return state.task_queue.get_result().clone();
        }

        Vec::new()
    }

    pub async fn search(&self, query: String) -> Result<Vec<SearchResult>, Error> {
        let (tx, rx) = oneshot::channel();
        self.command_tx
            .send(BrowserCommand::Search(query, tx))
            .await
            .map_err(|_| Error::ChannelClosed)?;
        rx.await.map_err(|_| Error::ResponseChannelClosed)
    }

    pub async fn keep_alive(&self) -> Result<(), Error> {
        self.command_tx
            .send(BrowserCommand::KeepAlive)
            .await
            .map_err(|_| Error::ChannelClosed)
    }

    pub async fn shutdown(self) -> Result<(), Error> {
        self.command_tx
            .send(BrowserCommand::Shutdown)
            .await
            .map_err(|_| Error::ChannelClosed)?;

        match self.handle.await {
            Ok(_) => Ok(()),
            Err(e) => Err(Error::JoinError(format!("{}", e))),
        }
    }
}
