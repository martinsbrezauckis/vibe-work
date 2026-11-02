//! The person's own pointer presses and releases on one browser view, as the host observes them, for checked
//! native clicks: refused while the person is clicking, never confirmed when the person clicked meanwhile.
//! Engines feed it from native window events (never from page script) and leave out the input they submit
//! themselves. Depends on std only. Called by engine adapters and `NativeClickLease`.

use std::sync::{
    Arc, Mutex, PoisonError, Weak,
    atomic::{AtomicU8, Ordering},
};
use std::time::{Duration, Instant};

/// Refusal before any input is sent.
pub const PERSON_BUSY: &str =
    "Not run: the person is using the browser right now; try again in a moment";
/// Result once input was sent and the person pressed or released a pointer button in the view meanwhile.
pub const PERSON_CLICKED: &str = "native input was sent; the person clicked in the browser at the same time, so earlier refs may no longer match; read the page again";
const UNOBSERVED: &str = "native input was sent; the person's pointer could not be observed";

const IDLE: u8 = 0;
const WATCHING: u8 = 1;
const DISTURBED: u8 = 2;

/// One checked click, from arming (before any receipt listener exists) to its final collection.
/// Created by the preview service; only the engine's native watch arms or disturbs it.
#[derive(Debug, Default)]
pub struct PersonInterval(AtomicU8);

impl PersonInterval {
    /// Whether the click may still be confirmed: armed by the engine and no press or release of the person's
    /// observed since.
    ///
    /// # Errors
    /// The agent-facing text when the person clicked, or the interval was never watched.
    pub fn undisturbed(&self) -> Result<(), &'static str> {
        match self.0.load(Ordering::SeqCst) {
            WATCHING => Ok(()),
            DISTURBED => Err(PERSON_CLICKED),
            _ => Err(UNOBSERVED),
        }
    }
}

/// Watch state of one native view, kept for the view's lifetime.
#[derive(Debug, Default)]
pub struct PersonPointer(Mutex<Watch>);

#[derive(Debug, Default)]
struct Watch {
    last: Option<Instant>,
    armed: Vec<Weak<PersonInterval>>,
}

impl PersonPointer {
    fn watch(&self) -> std::sync::MutexGuard<'_, Watch> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Records one press or release by the person in this view. The engine never calls this for its own input.
    pub fn observed(&self) {
        let mut watch = self.watch();
        watch.last = Some(Instant::now());
        watch.armed.retain(|interval| {
            interval.upgrade().is_some_and(|interval| {
                interval.0.store(DISTURBED, Ordering::SeqCst);
                true
            })
        });
    }

    /// Starts watching `interval`; called on the view's native thread before the receipt is armed.
    pub fn arm(&self, interval: &Arc<PersonInterval>) {
        let mut watch = self.watch();
        let _ = interval
            .0
            .compare_exchange(IDLE, WATCHING, Ordering::SeqCst, Ordering::SeqCst);
        watch.armed.retain(|interval| interval.strong_count() > 0);
        watch.armed.push(Arc::downgrade(interval));
    }

    /// Checked immediately before the engine submits input. `held`: the platform reports a pointer button down
    /// over this view now. `recent`: how long after the person's last press or release input still waits.
    ///
    /// # Errors
    /// [`PERSON_BUSY`] while held, within `recent`, or once the person pressed or released since arming.
    pub fn admit(
        &self,
        interval: &PersonInterval,
        held: bool,
        recent: Duration,
    ) -> Result<(), String> {
        let watch = self.watch();
        if held
            || watch.last.is_some_and(|last| last.elapsed() < recent)
            || interval.0.load(Ordering::SeqCst) != WATCHING
        {
            return Err(PERSON_BUSY.to_owned());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_press_while_armed_disturbs_and_refuses() {
        let pointer = PersonPointer::default();
        let interval = Arc::new(PersonInterval::default());
        assert!(interval.undisturbed().is_err());
        pointer.arm(&interval);
        assert_eq!(pointer.admit(&interval, false, Duration::ZERO), Ok(()));
        assert_eq!(interval.undisturbed(), Ok(()));
        pointer.observed();
        assert_eq!(interval.undisturbed(), Err(PERSON_CLICKED));
        assert_eq!(
            pointer.admit(&interval, false, Duration::ZERO),
            Err(PERSON_BUSY.to_owned())
        );
    }

    #[test]
    fn held_recent_or_unarmed_refuses_and_a_later_interval_starts_clean() {
        let pointer = PersonPointer::default();
        let unarmed = PersonInterval::default();
        assert!(pointer.admit(&unarmed, false, Duration::ZERO).is_err());
        pointer.observed();
        let interval = Arc::new(PersonInterval::default());
        pointer.arm(&interval);
        assert_eq!(interval.undisturbed(), Ok(()));
        assert!(
            pointer
                .admit(&interval, false, Duration::from_secs(60))
                .is_err()
        );
        assert!(pointer.admit(&interval, true, Duration::ZERO).is_err());
        assert_eq!(pointer.admit(&interval, false, Duration::ZERO), Ok(()));
    }
}
