use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

pub(crate) struct LatestRequestWorker<I, O> {
    generation: Arc<AtomicU64>,
    pending: Arc<Mutex<Option<(u64, I)>>>,
    wake: async_channel::Sender<()>,
    results: async_channel::Receiver<(u64, O)>,
}

impl<I: Send + 'static, O: Send + 'static> LatestRequestWorker<I, O> {
    pub(crate) fn spawn(build: impl Fn(I) -> O + Send + 'static) -> Self {
        let generation = Arc::new(AtomicU64::new(0));
        let pending = Arc::new(Mutex::new(None::<(u64, I)>));
        let (wake, wake_rx) = async_channel::bounded(1);
        let (result_tx, results) = async_channel::unbounded();
        let worker_pending = pending.clone();
        std::thread::spawn(move || {
            while wake_rx.recv_blocking().is_ok() {
                let request = worker_pending
                    .lock()
                    .expect("Agenda worker pending slot poisoned")
                    .take();
                if let Some((generation, input)) = request {
                    let output = build(input);
                    let _ = result_tx.send_blocking((generation, output));
                }
            }
        });
        Self {
            generation,
            pending,
            wake,
            results,
        }
    }

    pub(crate) fn submit(&self, input: I) -> u64 {
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        *self
            .pending
            .lock()
            .expect("Agenda worker pending slot poisoned") = Some((generation, input));
        let _ = self.wake.try_send(());
        generation
    }

    pub(crate) fn try_latest(&self) -> Option<O> {
        let latest = self.generation.load(Ordering::Acquire);
        let mut accepted = None;
        while let Ok((generation, output)) = self.results.try_recv() {
            if generation == latest {
                accepted = Some(output);
            }
        }
        accepted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_latest_generation_can_publish() {
        let worker = LatestRequestWorker::spawn(|value: usize| value * 2);
        worker.submit(1);
        worker.submit(2);
        for _ in 0..100 {
            if let Some(value) = worker.try_latest() {
                assert_eq!(value, 4);
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        panic!("latest Agenda request was not published");
    }
}
