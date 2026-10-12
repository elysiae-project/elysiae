use std::sync::OnceLock;

use std::future::Future;

use tokio::runtime::{Builder, Runtime};

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

pub fn runtime() -> &'static Runtime {
    RUNTIME.get_or_init(|| {
        Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("Failed to initialise the Tokio runtime")
    })
}

pub fn spawn<F>(future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    runtime().spawn(future)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_is_initialized_once_and_executes_tasks() {
        let first = runtime();
        let second = runtime();
        assert!(std::ptr::eq(first, second));
        assert_eq!(first.block_on(async { 2 + 2 }), 4);
    }
}
