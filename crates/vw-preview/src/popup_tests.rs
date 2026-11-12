//! Pure popup close transitions cover exact opener identity, permission fencing and one-time counts.
//! The service calls this same transition only after confirmed native teardown.
use super::*;
use crate::PageTabOwner;
use crate::state::{Entry, SessionTabs, State};

#[test]
fn closed_popup_never_selects_a_changed_or_unavailable_opener() {
    for change in [
        "none",
        "document",
        "url",
        "ownership",
        "closed",
        "stop",
        "unselected",
    ] {
        let mut state = State::default();
        let mut tabs = SessionTabs::default();
        for page in ["opener", "popup", "unrelated"] {
            state.page_owners.insert(page.into(), "chat".into());
            state.entries.insert(page.into(), Entry::new(page));
            tabs.web.push(page.into());
            tabs.owners.insert(page.into(), PageTabOwner::Agent);
        }
        tabs.agent = Some("popup".into());
        state.tabs.insert("chat".into(), tabs);
        let fingerprint = crate::tabs::review_fingerprint(&state, "chat", "opener").unwrap();
        state.tabs.get_mut("chat").unwrap().popups.links.insert(
            "popup".into(),
            PopupOpener {
                page: "opener".into(),
                fingerprint,
                allowed: Arc::new(move || change != "stop"),
            },
        );
        match change {
            "document" => state.entries.get_mut("opener").unwrap().reset_console(),
            "url" => {
                state.entries.get_mut("opener").unwrap().url =
                    Some("https://changed.test/".parse().unwrap());
            }
            "ownership" => {
                state
                    .tabs
                    .get_mut("chat")
                    .unwrap()
                    .owners
                    .insert("opener".into(), PageTabOwner::User);
            }
            "closed" => {
                state.page_owners.remove("opener");
            }
            "unselected" => state.tabs.get_mut("chat").unwrap().agent = Some("unrelated".into()),
            _ => (),
        }
        let result = closed(&mut state, "chat", "popup");
        assert_eq!(
            result,
            match change {
                "none" => Some(Return::Available("opener".into())),
                "unselected" => None,
                _ => Some(Return::Unavailable),
            },
            "{change}"
        );
        assert_eq!(state.tabs["chat"].popups.closed, 1);
        assert_eq!(closed(&mut state, "chat", "popup"), None);
        assert_eq!(state.tabs["chat"].popups.closed, 1);
    }
}
