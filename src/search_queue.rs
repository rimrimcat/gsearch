use chromiumoxide::Page;
use tokio::sync::{Mutex, Notify};

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;
use std::time::Instant;

use crate::search::SearchResult;
use crate::search::SearchTask;

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
