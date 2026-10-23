use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[derive(Clone, Default)]
pub(crate) struct CancelToken(Arc<AtomicBool>);
impl CancelToken {
    pub(crate) fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub(crate) fn cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
    pub(super) async fn wait(&self) {
        while !self.cancelled() {
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    }
}
