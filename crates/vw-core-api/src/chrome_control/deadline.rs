//! One transport deadline for a browser call, including nested gateway routes and helper requests.
//! Task-local scope keeps concurrent chats independent; pre-tool person consent remains outside this short budget.
use std::{future::Future, time::Duration};
use tokio::time::Instant;

// Leave time for the receipt to reach the agent's 50-second transport deadline.
const BUDGET: Duration = Duration::from_secs(45);
pub(super) const EXPIRED: &str = "Outcome unknown: this browser step reached its time limit before a result was confirmed. Check browser status and read the page before deciding what to do next; do not replay writes.";

tokio::task_local! {
    static UNTIL: Instant;
}

pub(super) fn until(maximum: Duration) -> Instant {
    let local = Instant::now() + maximum;
    UNTIL
        .try_with(|deadline| local.min(*deadline))
        .unwrap_or(local)
}

pub(super) fn remaining(maximum: Duration) -> Duration {
    until(maximum).saturating_duration_since(Instant::now())
}

pub(super) fn expired() -> bool {
    UNTIL
        .try_with(|deadline| Instant::now() >= *deadline)
        .unwrap_or(false)
}

pub(super) fn run<T>(
    future: impl Future<Output = Result<T, String>>,
) -> impl Future<Output = Result<T, String>> {
    // Box before constructing this async state: callers do not retain a large browser future inline.
    let future = Box::pin(future);
    async move {
        // Nested status lookups, retries and batch steps spend the original call's remaining time.
        if UNTIL.try_with(|_| ()).is_ok() {
            return future.await;
        }
        let deadline = Instant::now() + BUDGET;
        UNTIL
            .scope(deadline, async {
                tokio::time::timeout_at(deadline, future)
                    .await
                    .unwrap_or_else(|_| Err(EXPIRED.into()))
            })
            .await
    }
}

#[cfg(all(test, feature = "your-edge"))]
pub(super) async fn test_scope<T>(until: Instant, future: impl Future<Output = T>) -> T {
    UNTIL.scope(until, future).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn nested_waits_never_reset_the_call_deadline() {
        let start = Instant::now();
        let result: Result<(), String> = run(async {
            tokio::time::sleep(Duration::from_secs(40)).await;
            assert_eq!(remaining(Duration::from_secs(10)), Duration::from_secs(5));
            run(async {
                tokio::time::sleep(Duration::from_secs(10)).await;
                Ok(())
            })
            .await
        })
        .await;
        assert!(result.unwrap_err().contains("do not replay writes"));
        assert_eq!(start.elapsed(), BUDGET);
    }
}

#[cfg(test)]
mod size_tests {
    #[test]
    #[allow(
        clippy::large_stack_arrays,
        reason = "a deliberately large captured future checks the gateway's bounded async state"
    )]
    fn deadline_wrapper_keeps_large_browser_futures_off_its_inline_state() {
        let payload = [1_u8; 32_768];
        let future = super::run(async move {
            tokio::task::yield_now().await;
            std::hint::black_box(payload);
            Ok(())
        });
        assert!(
            std::mem::size_of_val(&future) < 1024,
            "deadline state grew to {} bytes",
            std::mem::size_of_val(&future)
        );
    }
}
