use std::{
    future::Future,
    sync::{Arc, Mutex},
};
use tokio::task::JoinHandle;

#[derive(Default)]
struct State {
    closed: bool,
    exhausted: bool,
    tasks: Vec<JoinHandle<()>>,
}

// Track hyper's internal HTTP/2 tasks as well as its connection driver. A
// closed executor drops new work; cleanup aborts and awaits every owned task.
#[derive(Clone, Default)]
pub(crate) struct Executor(Arc<Mutex<State>>);

impl<F> hyper::rt::Executor<F> for Executor
where
    F: Future + Send + 'static,
    F::Output: Send,
{
    fn execute(&self, future: F) {
        self.spawn(async move {
            let _ = future.await;
        });
    }
}

impl Executor {
    pub fn spawn(&self, future: impl Future<Output = ()> + Send + 'static) {
        let mut state = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
        if !state.closed {
            if state.tasks.len() >= 64 {
                state.exhausted = true;
                return;
            }
            state.tasks.push(tokio::spawn(future));
        }
    }
    pub fn exhausted(&self) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .exhausted
    }
    pub async fn close(&self) {
        let tasks = {
            let mut state = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
            state.closed = true;
            std::mem::take(&mut state.tasks)
        };
        for task in &tasks {
            task.abort();
        }
        for task in tasks {
            let _ = task.await;
        }
    }
}
