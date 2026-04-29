#![warn(clippy::missing_errors_doc, clippy::result_large_err)]

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};
use std::thread;
use std::time::{Duration, Instant};

struct MeasurableFuture<Fut> {
    inner_future: Fut,
    started_at: Option<Instant>,
}

impl<Fut> MeasurableFuture<Fut> {
    fn new(inner_future: Fut) -> Self {
        Self {
            inner_future,
            started_at: None,
        }
    }
}

impl<Fut> Future for MeasurableFuture<Fut>
where
    Fut: Future,
{
    type Output = Fut::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };

        if this.started_at.is_none() {
            this.started_at = Some(Instant::now());
        }

        let inner = unsafe { Pin::new_unchecked(&mut this.inner_future) };

        match inner.poll(cx) {
            Poll::Ready(output) => {
                if let Some(started_at) = this.started_at {
                    println!("Час виконання inner_future: {:?}", started_at.elapsed());
                }

                Poll::Ready(output)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

struct DelayFuture {
    shared_state: Arc<Mutex<DelayState>>,
}

struct DelayState {
    completed: bool,
    waker: Option<Waker>,
}

impl DelayFuture {
    fn new(milliseconds: u64) -> Self {
        let shared_state = Arc::new(Mutex::new(DelayState {
            completed: false,
            waker: None,
        }));

        let thread_state = Arc::clone(&shared_state);

        thread::spawn(move || {
            thread::sleep(Duration::from_millis(milliseconds));

            let mut state = thread_state.lock().expect("Mutex poisoned");
            state.completed = true;

            if let Some(waker) = state.waker.take() {
                waker.wake();
            }
        });

        Self { shared_state }
    }
}

impl Future for DelayFuture {
    type Output = &'static str;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self.shared_state.lock().expect("Mutex poisoned");

        if state.completed {
            Poll::Ready("DelayFuture completed")
        } else {
            state.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

struct ThreadWaker {
    thread: thread::Thread,
}

impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) {
        self.thread.unpark();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.thread.unpark();
    }
}

fn block_on<Fut>(future: Fut) -> Fut::Output
where
    Fut: Future,
{
    let waker = Waker::from(Arc::new(ThreadWaker {
        thread: thread::current(),
    }));

    let mut context = Context::from_waker(&waker);
    let mut future = Box::pin(future);

    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => thread::park(),
        }
    }
}

fn main() {
    let delay_future = DelayFuture::new(2000);
    let measurable_future = MeasurableFuture::new(delay_future);

    let result = block_on(measurable_future);

    println!("Результат: {result}");
}