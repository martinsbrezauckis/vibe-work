//! An in-flight callback's reservation, owned by the card/automatic answer until Vibe confirms it.
//! Depends on the registry mutex; completion and task cancellation each leave exactly one honest terminal record.
use super::{AnsweredBy, Arc, Outcome, Service};

pub(crate) struct Delivery {
    service: Arc<Service>,
    id: String,
    by: AnsweredBy,
    finished: bool,
}

impl Service {
    /// Reserves an Ask against another click and callback-close notifications while sending.
    pub(crate) fn reserve(self: &Arc<Self>, id: &str, by: AnsweredBy) -> Option<Delivery> {
        let mut state = self.lock();
        let entry = state.open.get_mut(id)?;
        if entry.delivering {
            return None;
        }
        entry.delivering = true;
        Some(Delivery {
            service: self.clone(),
            id: id.into(),
            by,
            finished: false,
        })
    }
}

impl Delivery {
    /// Only a successful callback/result may finish with the proposed decision.
    pub(crate) fn confirmed(mut self, outcome: Outcome) {
        self.service.settle_inner(&self.id, outcome, true);
        self.finished = true;
    }
}

impl Drop for Delivery {
    fn drop(&mut self) {
        if !self.finished {
            self.service
                .settle_inner(&self.id, Outcome::AnswerNotConfirmed { by: self.by }, true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vw_ask::{Choice, Scope};

    #[tokio::test(start_paused = true)]
    async fn a_reserved_answer_is_recorded_only_after_confirmation_after_two_hours() {
        for outcome in [
            Outcome::Allowed {
                by: AnsweredBy::Person,
                scope: Scope::Once,
            },
            Outcome::Refused {
                by: AnsweredBy::Person,
                note: None,
            },
        ] {
            let service = Service::new(
                Arc::new(super::super::SystemAskClock::default()),
                tokio::sync::broadcast::channel(16).0,
            );
            let mut handle = service.open_callback(
                "approval",
                Some("s".into()),
                vec![Choice::Once, Choice::No],
                None,
                false,
            );
            let delivery = service.reserve(&handle.ask.id, AnsweredBy::Person).unwrap();
            assert!(
                service
                    .reserve(&handle.ask.id, AnsweredBy::Person)
                    .is_none()
            );
            tokio::time::advance(std::time::Duration::from_secs(601)).await;
            service.cancel_session(Some("s"), "closed during delivery");
            assert_eq!(service.records(), Vec::new());
            delivery.confirmed(outcome.clone());
            assert_eq!(handle.closed().await, outcome);
            assert_eq!(service.records().len(), 1);
            assert_eq!(service.records()[0].outcome, outcome);
        }
    }

    #[tokio::test]
    async fn dropping_a_delivery_records_uncertainty_and_never_releases_it_for_retry() {
        let service = Service::new(
            Arc::new(super::super::SystemAskClock::default()),
            tokio::sync::broadcast::channel(16).0,
        );
        let handle = service.open_callback("approval", None, vec![Choice::Once], None, false);
        drop(service.reserve(&handle.ask.id, AnsweredBy::Person).unwrap());
        assert_eq!(
            service.records()[0].outcome,
            Outcome::AnswerNotConfirmed {
                by: AnsweredBy::Person
            }
        );
        assert!(
            service
                .reserve(&handle.ask.id, AnsweredBy::Person)
                .is_none()
        );
    }
}
