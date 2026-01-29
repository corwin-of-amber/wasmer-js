use futures::task::AtomicWaker;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug)]
pub(crate) struct AsyncEvent {
    waker: AtomicWaker,
    set: AtomicBool,
}

impl AsyncEvent {
    pub fn new() -> Self {
        Self { waker: AtomicWaker::new(), set: AtomicBool::new(false) }
    }

    pub fn is_set(&self) -> bool {
        self.set.load(Ordering::Acquire)
    }

    pub fn set(&self) {
        self.set.store(true, Ordering::Release);
        self.waker.wake();
    }

    pub fn reset(&self) {
        self.set.store(false, Ordering::Release);
    }

    pub fn poll(&self, cx: &mut std::task::Context<'_>) -> bool {
        if self.is_set() {
            true
        }
        else {
            self.waker.register(cx.waker());
            false
        }
    }
}
