//! Fault switches of the test page engine that model what other engines answer, through `PageEngine`'s frame answers.
//! Called by the all-frames evaluation in `mod.rs`.

use super::PageEngine;
use std::sync::atomic::{AtomicU64, Ordering};

/// Takes one from `counter` when it is above zero (`u64::MAX` never runs out); whether it did.
pub(super) fn take_one(counter: &AtomicU64) -> bool {
    let left = counter.load(Ordering::Relaxed);
    left == u64::MAX
        || (left > 0
            && counter
                .compare_exchange(left, left - 1, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok())
}

impl PageEngine {
    /// While `detached_child_evaluations` lasts, an all-frames evaluation (reads, the click's frame registry and its
    /// receipt) also gets an answer from a removed child frame's handle (`{"detached":true}`), as `WebKit` on macOS
    /// answers for a while after the frame left the page.
    pub(super) fn add_detached_child(&self, answers: &mut Vec<vw_preview::FrameAnswer>) {
        if take_one(&self.detached_child_evaluations)
            && let Some(child) = answers.iter().find(|a| !a.main).cloned()
        {
            answers.push(vw_preview::FrameAnswer {
                id: format!("{}-removed", child.id),
                value: vw_preview::DETACHED_FRAME.to_owned(),
                ..child
            });
        }
    }
}
