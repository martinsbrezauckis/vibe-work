//! Chrome's legacy UUID alias at the helper boundary. Trusted status selects the wire format; Edge stays scoped.
use serde_json::Value;
use std::sync::{Mutex, PoisonError};

#[derive(Default)]
pub(super) struct ProfileIds(Mutex<Option<bool>>);
impl ProfileIds {
    pub(super) fn observe(&self, data: &Value) {
        let profiles = data["profiles"].as_array();
        let scoped_id = |id: &str| {
            id.strip_prefix("chrome:")
                .or_else(|| id.strip_prefix("edge:"))
                .is_some_and(super::enroll::uuid_shape)
        };
        let scoped = data["profileId"].as_str().is_some_and(scoped_id)
            || profiles
                .into_iter()
                .flatten()
                .any(|profile| profile["profileId"].as_str().is_some_and(scoped_id));
        if !scoped && profiles.is_none_or(Vec::is_empty) {
            return;
        }
        let mut current = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        *current = Some(current.unwrap_or(false) || scoped);
    }
    pub(super) fn clear(&self) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
    pub(super) fn needs_status(&self, args: &Value) -> bool {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_none()
            && bare(args)
    }
    pub(super) fn normalize(&self, args: &mut Value) {
        if *self.0.lock().unwrap_or_else(PoisonError::into_inner) != Some(true) {
            return;
        }
        for path in ["/profileId", "/_tabContext/profileId"] {
            if let Some(id) = args.pointer_mut(path)
                && let Some(raw) = id.as_str().filter(|id| super::enroll::uuid_shape(id))
            {
                *id = Value::String(format!("chrome:{}", raw.to_ascii_lowercase()));
            }
        }
    }
}

fn bare(args: &Value) -> bool {
    ["/profileId", "/_tabContext/profileId"].iter().any(|path| {
        args.pointer(path)
            .and_then(Value::as_str)
            .is_some_and(super::enroll::uuid_shape)
    })
}

/// Old saved bindings and new helper labels can name the same Chrome profile; never an Edge profile.
pub(super) fn same(left: &str, right: &str) -> bool {
    fn chrome(id: &str) -> Option<&str> {
        let raw = id.strip_prefix("chrome:").unwrap_or(id);
        super::enroll::uuid_shape(raw).then_some(raw)
    }
    left == right
        || chrome(left)
            .zip(chrome(right))
            .is_some_and(|(left, right)| left.eq_ignore_ascii_case(right))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    const RAW: &str = "12345678-1234-1234-1234-123456789abc";
    #[test]
    fn aliases_follow_the_current_helper_and_never_match_edge() {
        let ids = ProfileIds::default();
        ids.observe(&json!({"profileId":format!("chrome:{RAW}")}));
        let mut args = json!({"profileId":RAW,"args":{"profileId":RAW}});
        ids.normalize(&mut args);
        assert_eq!(args["profileId"], format!("chrome:{RAW}"));
        assert_eq!(
            args["args"]["profileId"], RAW,
            "page data is not a browser identity"
        );
        assert!(same(RAW, &format!("chrome:{RAW}")));
        assert!(!same(RAW, &format!("edge:{RAW}")));
        ids.clear();
        ids.observe(&json!({"profiles":[{"profileId":RAW}]}));
        let mut args = json!({"profileId":RAW});
        ids.normalize(&mut args);
        assert_eq!(
            args["profileId"], RAW,
            "a restarted legacy helper retains its wire format"
        );
    }
}
