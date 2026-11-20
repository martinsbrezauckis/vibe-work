//! The shared permission gate's order of checks ([`super`]), with a record that changes at one step.
//!
//! PREDICT (written before the gate): commands run with a secret had their own consent path, whose later run read
//! every secret value first and compared the permissions only afterwards. With one gate,
//! `a_permission_changed_after_the_consent_reads_no_value` and `a_pause_after_the_consent_reads_no_value` hold for
//! both callers: no value is read on a record that changed after the person answered. The other tests pin one step
//! each: a gate that leaves that step out fails it.

use std::future::{Future, ready};
use std::sync::Mutex;

use super::{Policy, grant, read, recheck, value};

#[derive(Clone, Debug, PartialEq)]
struct Record {
    allowed: bool,
    version: u32,
}

#[derive(Default)]
struct State {
    allowed: bool,
    version: u32,
    paused: bool,
    checks: u32,
    values: u32,
    confirmed: u32,
}

#[derive(Default)]
struct Fake(Mutex<State>);

impl Fake {
    fn with(allowed: bool) -> Self {
        let fake = Self::default();
        fake.0.lock().unwrap().allowed = allowed;
        fake
    }
    fn set(&self, change: impl FnOnce(&mut State)) {
        change(&mut self.0.lock().unwrap());
    }
    fn counts(&self) -> (u32, u32) {
        let state = self.0.lock().unwrap();
        (state.checks, state.values)
    }
}

impl Policy for Fake {
    type Record = Record;
    type Answer = bool;
    type Refusal = &'static str;
    type Value = &'static str;

    fn check(&self) -> impl Future<Output = Result<Record, &'static str>> + Send {
        let mut state = self.0.lock().unwrap();
        state.checks += 1;
        ready(if state.paused {
            Err("paused")
        } else {
            Ok(Record {
                allowed: state.allowed,
                version: state.version,
            })
        })
    }
    fn permits(&self, record: &Record, answer: Option<&bool>) -> bool {
        record.allowed || answer.is_some()
    }
    fn same_as_shown(&self, shown: &Record, now: &Record) -> bool {
        shown == now
    }
    fn confirmed(
        &self,
        mut record: Record,
        always: &bool,
    ) -> impl Future<Output = Result<Record, &'static str>> + Send {
        if *always {
            let mut state = self.0.lock().unwrap();
            state.confirmed += 1;
            state.allowed = true;
            record.allowed = true;
        }
        ready(Ok(record))
    }
    fn not_allowed(&self) -> &'static str {
        "not allowed"
    }
    fn changed(&self) -> &'static str {
        "changed"
    }
    fn value(
        &self,
        _granted: &Record,
    ) -> impl Future<Output = Result<&'static str, &'static str>> + Send {
        self.0.lock().unwrap().values += 1;
        ready(Ok("fixture-value-not-real"))
    }
}

#[tokio::test]
async fn a_standing_grant_asks_nothing_and_reads_between_two_checks() {
    let fake = Fake::with(true);
    let granted = grant(&fake, |_| async {
        panic!("no dialog for a standing grant")
    })
    .await
    .unwrap();
    assert!(granted.answer().is_none());
    let (_, value) = value(&fake, &granted).await.unwrap();
    assert_eq!(value, "fixture-value-not-real");
    assert_eq!(fake.counts(), (2, 1), "check, value, check");
}

#[tokio::test]
async fn a_no_reads_nothing() {
    let fake = Fake::with(false);
    let refused = grant(&fake, |_| async { Err("declined") }).await;
    assert_eq!(refused.err(), Some("declined"));
    assert_eq!(fake.counts(), (1, 0));
}

#[tokio::test]
async fn a_change_during_the_dialog_refuses_before_any_value() {
    let fake = Fake::with(false);
    let refused = grant(&fake, |_| async {
        fake.set(|s| s.version += 1);
        Ok(false)
    })
    .await;
    assert_eq!(refused.err(), Some("changed"));
    assert_eq!(fake.counts(), (2, 0));
}

#[tokio::test]
async fn a_pause_during_the_dialog_refuses() {
    let fake = Fake::with(false);
    let refused = grant(&fake, |_| async {
        fake.set(|s| s.paused = true);
        Ok(false)
    })
    .await;
    assert_eq!(refused.err(), Some("paused"));
    assert_eq!(fake.counts().1, 0);
}

#[tokio::test]
async fn an_always_answer_is_recorded_once_after_the_fresh_check() {
    let fake = Fake::with(false);
    let granted = grant(&fake, |_| async { Ok(true) }).await.unwrap();
    assert!(
        granted.record().allowed,
        "the recorded Allowed is the granted record"
    );
    assert_eq!(fake.0.lock().unwrap().confirmed, 1);
    let (_, value) = read(&fake, &granted).await.unwrap();
    assert_eq!(value, "fixture-value-not-real");
}

#[tokio::test]
async fn a_permission_changed_after_the_consent_reads_no_value() {
    let fake = Fake::with(false);
    let granted = grant(&fake, |_| async { Ok(false) }).await.unwrap();
    fake.set(|s| s.version += 1);
    assert_eq!(read(&fake, &granted).await.err(), Some("changed"));
    assert_eq!(fake.counts().1, 0, "no value read on a changed record");
    // A standing grant that is taken back (Allowed -> Ask each time) refuses the same way.
    let fake = Fake::with(true);
    let granted = grant(&fake, |_| async { Ok(false) }).await.unwrap();
    fake.set(|s| s.allowed = false);
    assert_eq!(read(&fake, &granted).await.err(), Some("not allowed"));
    assert_eq!(fake.counts().1, 0);
}

#[tokio::test]
async fn a_pause_after_the_consent_reads_no_value() {
    let fake = Fake::with(true);
    let granted = grant(&fake, |_| async { Ok(false) }).await.unwrap();
    fake.set(|s| s.paused = true);
    assert_eq!(read(&fake, &granted).await.err(), Some("paused"));
    assert_eq!(fake.counts().1, 0);
    assert_eq!(recheck(&fake, &granted).await.err(), Some("paused"));
}

#[tokio::test]
async fn a_change_while_the_value_is_read_refuses() {
    struct Racing(Fake);
    impl Policy for Racing {
        type Record = Record;
        type Answer = bool;
        type Refusal = &'static str;
        type Value = &'static str;
        async fn check(&self) -> Result<Record, &'static str> {
            self.0.check().await
        }
        fn permits(&self, record: &Record, answer: Option<&bool>) -> bool {
            self.0.permits(record, answer)
        }
        fn same_as_shown(&self, shown: &Record, now: &Record) -> bool {
            self.0.same_as_shown(shown, now)
        }
        fn not_allowed(&self) -> &'static str {
            self.0.not_allowed()
        }
        fn changed(&self) -> &'static str {
            self.0.changed()
        }
        async fn value(&self, granted: &Record) -> Result<&'static str, &'static str> {
            let value = self.0.value(granted).await;
            self.0.set(|s| s.version += 1);
            value
        }
    }
    let racing = Racing(Fake::with(true));
    let granted = grant(&racing, |_| async { Ok(false) }).await.unwrap();
    assert_eq!(read(&racing, &granted).await.err(), Some("changed"));
}
