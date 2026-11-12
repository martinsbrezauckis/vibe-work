//! Pauses the batch work deadline while its hook waits for a person's answer.
//! Used only by batch execution and helper-discovered capability approval.
//! Depends on: Tokio watch and the batch deadline.
//! Called by: batch execution and discovered-capability approval.
use std::{future::Future, time::Duration};
use tokio::{sync::watch, time::Instant};

#[derive(Clone)]
pub(super) struct Clock(watch::Sender<bool>);

impl Default for Clock {
    fn default() -> Self {
        Self(watch::channel(false).0)
    }
}

pub(super) struct Waiting(Clock);

impl Drop for Waiting {
    fn drop(&mut self) {
        self.0.0.send_replace(false);
    }
}

impl Clock {
    pub(super) fn pause(&self) -> Waiting {
        self.0.send_replace(true);
        Waiting(self.clone())
    }

    pub(super) async fn until<F: Future>(
        &self,
        mut deadline: Instant,
        future: F,
    ) -> (Result<F::Output, ()>, Duration) {
        let mut state = self.0.subscribe();
        let mut waited = Duration::ZERO;
        tokio::pin!(future);
        loop {
            if *state.borrow_and_update() {
                let started = Instant::now();
                let result = tokio::select! {
                    biased;
                    result = &mut future => Some(result),
                    _ = state.changed() => None,
                };
                let elapsed = started.elapsed();
                waited += elapsed;
                deadline += elapsed;
                if let Some(result) = result {
                    return (Ok(result), waited);
                }
            } else {
                tokio::select! {
                    biased;
                    result = &mut future => return (Ok(result), waited),
                    _ = state.changed() => {},
                    () = tokio::time::sleep_until(deadline) => return (Err(()), waited),
                }
            }
        }
    }
}
