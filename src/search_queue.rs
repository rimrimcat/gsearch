use bincode::{Decode, Encode};
use chromiumoxide::Page;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::{Mutex, Notify};

use std::collections::VecDeque;
use std::fs;
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;
use std::time::Instant;

use crate::browser_utils::BrowserWrapper;
use crate::browser_utils::PageWrapper;
use crate::browser_utils::connect_to_browser;
use crate::browser_utils::make_new_tab;
use crate::search::SearchResult;
use crate::search::SearchTask;

use bincode::config::standard;
use bincode::serde::{decode_from_slice, encode_into_slice};

#[derive(Debug)]
pub struct QueuedSearchTask {
    pub task: SearchTask,
    pub id: u32,
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
                let start = Instant::now();

                let __last_task_id = qtask.id;

                // Extract the needed parameters
                let query = qtask.task.query;
                let args = qtask.task.args;

                let new_result = qtask.task.engine.search(&self.page, query, args).await;

                {
                    let mut curr_result = self.result.write().unwrap();
                    *curr_result = new_result;
                }

                self.last_finished_task_id
                    .store(__last_task_id, Ordering::SeqCst);

                #[cfg(debug_assertions)]
                println!("Search finished in {}ms", start.elapsed().as_millis());

                self.result_notify.notify_waiters();

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

    pub async fn stop(self) {
        {
            let mut queue = self.queue.lock().await;
            queue.clear();
        }

        if self.is_running.load(Ordering::SeqCst) {
            self.is_running.store(false, Ordering::SeqCst);
            self.result_notify.notified().await;
        }
        let _ = self.page.close().await;
    }
}

#[derive(Serialize, Deserialize, Encode, Decode, Debug)]
pub enum BrowserRequest {
    AddTask(SearchTask),
    WaitResultUpdate,
    GetResult,
    GetLastFinishedTaskId,
    KeepAlive,
    Shutdown,
    Test,
}

#[derive(Serialize, Deserialize, Encode, Decode, Debug)]
pub enum BrowserResponse {
    TaskId(u32),
    ResultUpdateComplete,
    SearchResults(Vec<SearchResult>),
    LastFinishedTaskId(u32),
    KeepAlive,
    Shutdown,
    Error(String),
    Test,
}

pub struct BrowserServer {
    pub browser_wrapper: BrowserWrapper,
    pub page_wrapper: PageWrapper,
    pub queue: SearchTaskQueue,
    pub last_activity: Instant,
    pub socket_path: std::path::PathBuf,
}

impl BrowserServer {
    pub async fn new(
        port: u16,
        evasion_scripts_path: Option<std::path::PathBuf>,
        socket_path: std::path::PathBuf,
    ) -> Self {
        let browser_wrapper = connect_to_browser(port).await.unwrap();
        let page_wrapper = make_new_tab(&browser_wrapper.browser, evasion_scripts_path.clone())
            .await
            .unwrap();
        let queue = SearchTaskQueue::new(page_wrapper.page.clone());

        Self {
            browser_wrapper,
            page_wrapper,
            queue,
            last_activity: Instant::now(),
            socket_path,
        }
    }

    async fn handle_request(&self, mut stream: UnixStream) {
        let mut buffer = [0u8; 1024];

        let _ = stream.read(&mut buffer).await.unwrap();
        let (req, _): (BrowserRequest, usize) = decode_from_slice(&buffer, standard()).unwrap();

        println!("Server received: {:?}", req);

        let resp = match req {
            BrowserRequest::AddTask(task) => {
                BrowserResponse::TaskId(self.queue.add_task(task).await)
            }
            BrowserRequest::WaitResultUpdate => BrowserResponse::ResultUpdateComplete,
            BrowserRequest::GetResult => BrowserResponse::SearchResults(self.queue.get_result()),
            BrowserRequest::GetLastFinishedTaskId => {
                BrowserResponse::LastFinishedTaskId(self.queue.get_last_finished_task_id())
            }
            BrowserRequest::KeepAlive => BrowserResponse::KeepAlive,
            BrowserRequest::Shutdown => BrowserResponse::Shutdown,
            BrowserRequest::Test => BrowserResponse::Test,
        };

        encode_into_slice(resp, &mut buffer, standard()).unwrap();

        stream.write_all(&buffer).await.unwrap();
    }

    pub async fn start(self) {
        match fs::remove_file(&self.socket_path) {
            Ok(_) => {
                println!("Removed existing socket file");
            }
            Err(_) => {}
        }

        let listener = tokio::net::UnixListener::bind(&self.socket_path).unwrap();
        loop {
            match listener.accept().await {
                Ok((stream, _addr)) => {
                    self.handle_request(stream).await;
                }
                Err(e) => {
                    eprintln!("Failed to accept connection: {}", e);
                }
            }
        }
    }
}

pub struct BrowserClient {
    pub socket_path: std::path::PathBuf,
}

impl BrowserClient {
    pub fn new(socket_path: std::path::PathBuf) -> Self {
        Self { socket_path }
    }

    pub async fn send_request(
        &self,
        req: BrowserRequest,
    ) -> Result<BrowserResponse, Box<dyn std::error::Error>> {
        let mut stream = UnixStream::connect(&self.socket_path).await?;
        let mut buffer = [0u8; 1024];

        encode_into_slice(req, &mut buffer, standard()).unwrap();

        stream.write_all(&buffer).await?;
        stream.readable().await?;
        stream.read(&mut buffer).await?;

        let (resp, _): (BrowserResponse, usize) = decode_from_slice(&buffer, standard()).unwrap();

        println!("Client received: {:?}", resp);

        Ok(resp)
    }

    pub async fn send_test(&self) -> Result<BrowserResponse, Box<dyn std::error::Error>> {
        let resp = self.send_request(BrowserRequest::Test).await?;
        Ok(resp)
    }

    pub async fn add_task(&self, task: SearchTask) -> Result<u32, Box<dyn std::error::Error>> {
        let resp = self.send_request(BrowserRequest::AddTask(task)).await?;
        match resp {
            BrowserResponse::TaskId(id) => Ok(id),
            _ => Err("Invalid response".into()),
        }
    }

    pub async fn wait_result_update(&self) -> Result<(), Box<dyn std::error::Error>> {
        let resp = self.send_request(BrowserRequest::WaitResultUpdate).await?;
        match resp {
            BrowserResponse::ResultUpdateComplete => Ok(()),
            _ => Err("Invalid response".into()),
        }
    }

    pub async fn get_result(&self) -> Result<Vec<SearchResult>, Box<dyn std::error::Error>> {
        let resp = self.send_request(BrowserRequest::GetResult).await?;
        match resp {
            BrowserResponse::SearchResults(result) => Ok(result),
            _ => Err("Invalid response".into()),
        }
    }

    pub async fn get_last_finished_task_id(&self) -> Result<u32, Box<dyn std::error::Error>> {
        let resp = self
            .send_request(BrowserRequest::GetLastFinishedTaskId)
            .await?;
        match resp {
            BrowserResponse::LastFinishedTaskId(id) => Ok(id),
            _ => Err("Invalid response".into()),
        }
    }

    pub async fn keep_alive(&self) -> Result<(), Box<dyn std::error::Error>> {
        let resp = self.send_request(BrowserRequest::KeepAlive).await?;
        match resp {
            BrowserResponse::KeepAlive => Ok(()),
            _ => Err("Invalid response".into()),
        }
    }

    pub async fn shutdown(&self) -> Result<(), Box<dyn std::error::Error>> {
        let resp = self.send_request(BrowserRequest::Shutdown).await?;
        match resp {
            BrowserResponse::Shutdown => Ok(()),
            _ => Err("Invalid response".into()),
        }
    }
}
