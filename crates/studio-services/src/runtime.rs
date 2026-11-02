//! Shared multi-thread tokio runtime for every async service.
//!
//! The UI never touches tokio directly: it calls [`spawn`], which runs the
//! future on this runtime and hands back a runtime-agnostic `Send` future
//! that any executor (e.g. gpui's) can await.

use std::{future::Future, sync::LazyLock};

use tokio::runtime::{Builder, Handle, Runtime};

static RUNTIME: LazyLock<Runtime> = LazyLock::new(|| {
    Builder::new_multi_thread()
        .enable_all()
        .thread_name("studio-services")
        .build()
        .expect("failed to start the studio-services tokio runtime")
});

/// Handle to the shared runtime (for code that needs `Handle::enter`).
pub fn handle() -> Handle {
    RUNTIME.handle().clone()
}

/// Runs `fut` on the shared runtime and returns a future resolving to its output.
///
/// The returned future does not need a tokio context to be polled. The task
/// starts immediately and keeps running even if the returned future is dropped.
/// If the task panics, awaiting the returned future resumes that panic.
pub fn spawn<F>(fut: F) -> impl Future<Output = F::Output> + Send + 'static
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let task = RUNTIME.spawn(fut);
    async move {
        match task.await {
            Ok(output) => output,
            Err(error) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
            Err(error) => panic!("studio-services task was cancelled: {error}"),
        }
    }
}

/// Blocks the current (non-runtime) thread until `fut` completes on the shared runtime.
///
/// Intended for `main` before any window opens (e.g. the auth CLI). Panics if
/// called from inside a tokio runtime.
pub fn block_on<F: Future>(fut: F) -> F::Output {
    RUNTIME.block_on(fut)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_result_is_awaitable_outside_tokio() {
        let value = futures_lite_block_on(spawn(async { 21 * 2 }));
        assert_eq!(value, 42);
    }

    #[test]
    fn spawn_propagates_panics_to_the_awaiting_future() {
        let result = std::panic::catch_unwind(|| {
            futures_lite_block_on(spawn(async { panic!("boom") }));
        });
        assert!(result.is_err());
    }

    /// Minimal executor that is not tokio, standing in for gpui's.
    fn futures_lite_block_on<F: Future>(fut: F) -> F::Output {
        use std::{
            pin::pin,
            sync::Arc,
            task::{Context, Poll, Wake, Waker},
            thread::{self, Thread},
        };

        struct ThreadWaker(Thread);
        impl Wake for ThreadWaker {
            fn wake(self: Arc<Self>) {
                self.0.unpark();
            }
        }

        let waker = Waker::from(Arc::new(ThreadWaker(thread::current())));
        let mut context = Context::from_waker(&waker);
        let mut fut = pin!(fut);
        loop {
            match fut.as_mut().poll(&mut context) {
                Poll::Ready(output) => return output,
                Poll::Pending => thread::park(),
            }
        }
    }
}
