//! Optional video job reservations. Shares the composer image count ceiling and counts prior turn images.
//! One active job per chat and two globally; reservations survive neither completion nor cancellation.
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Mutex, PoisonError},
};
use vw_video_types::{Code, Error};
#[derive(Default)]
struct Turn {
    id: String,
    spent: usize,
    active: bool,
}
#[derive(Default)]
pub(super) struct Budget {
    turns: Mutex<HashMap<String, Turn>>,
}
pub(super) struct Reservation<'a> {
    budget: &'a Budget,
    owner: String,
    image: bool,
    sent: bool,
}
impl Budget {
    pub(super) fn reserve(
        &self,
        owner: &str,
        state: &Value,
        image: bool,
    ) -> Result<Reservation<'_>, Error> {
        let (turn, observed) = usage(state)?;
        let mut turns = self.turns.lock().unwrap_or_else(PoisonError::into_inner);
        if turns.values().filter(|t| t.active).count() >= 2 {
            return Err(Error::new(Code::Busy, "two video jobs are active"));
        }
        let entry = turns.entry(owner.into()).or_default();
        if entry.active {
            return Err(Error::new(Code::Busy, "this chat already has a video job"));
        }
        if entry.id != turn {
            *entry = Turn {
                id: turn.into(),
                ..Turn::default()
            };
        }
        if image && entry.spent.saturating_add(observed) >= vw_gate::MAX_IMAGES {
            return Err(Error::new(
                Code::ImageAllowance,
                "this user turn's image allowance is used",
            ));
        }
        // Pending image deliveries may not yet appear in history. Conservatively count both.
        entry.spent += usize::from(image);
        entry.active = true;
        Ok(Reservation {
            budget: self,
            owner: owner.into(),
            image,
            sent: false,
        })
    }
    pub(super) fn forget(&self, owner: &str) {
        self.turns
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(owner);
    }
}
impl Reservation<'_> {
    pub(super) fn sent(&mut self, state: &Value, image: bool) -> Result<(), Error> {
        let (turn, observed) = usage(state)?;
        let turns = self
            .budget
            .turns
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let entry = turns
            .get(&self.owner)
            .ok_or(Error::new(Code::Cancelled, "video turn ended"))?;
        if entry.id != turn {
            return Err(Error::new(Code::Cancelled, "video turn ended"));
        }
        if image && !self.image {
            return Err(Error::new(Code::InvalidReply, "no image was reserved"));
        }
        if image && entry.spent.saturating_add(observed) > vw_gate::MAX_IMAGES {
            return Err(Error::new(
                Code::ImageAllowance,
                "this user turn's image allowance is used",
            ));
        }
        self.sent = image;
        Ok(())
    }
}
impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if let Some(t) = self
            .budget
            .turns
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(&self.owner)
        {
            t.active = false;
            if self.image && !self.sent {
                t.spent = t.spent.saturating_sub(1);
            }
        }
    }
}
fn usage(state: &Value) -> Result<(&str, usize), Error> {
    let history = state["history"]
        .as_array()
        .ok_or(Error::new(Code::NoImageTurn, "no user turn image budget"))?;
    let start = history
        .iter()
        .rposition(|e| e["type"] == "message" && e["role"] == "user" && e["source"] != "harness")
        .ok_or(Error::new(Code::NoImageTurn, "no user turn image budget"))?;
    let turn = history[start]["turnId"]
        .as_str()
        .ok_or(Error::new(Code::NoImageTurn, "no user turn image budget"))?;
    let observed = history[start..].iter().map(images).sum::<usize>();
    Ok((turn, observed))
}
fn images(value: &Value) -> usize {
    match value {
        Value::Object(map)
            if map.get("type").is_some_and(|v| v == "image") || map.contains_key("vwImage") =>
        {
            1
        }
        Value::Object(map) => map.values().map(images).sum(),
        Value::Array(list) => list.iter().map(images).sum(),
        _ => 0,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn no_cut_deliveries_refund_images_but_still_check_the_turn() {
        let b = Budget::default();
        let state =
            json!({"history":[{"type":"message","role":"user","turnId":"one","content":[]}]});
        for _ in 0..8 {
            let mut r = b.reserve("chat", &state, true).unwrap();
            r.sent(&state, false).unwrap();
        }
        let mut r = b.reserve("chat", &state, true).unwrap();
        let mut next = state.clone();
        next["history"][0]["turnId"] = json!("two");
        assert!(r.sent(&next, false).is_err());
    }
    #[test]
    fn images_arriving_during_decode_are_rechecked_before_sending() {
        let b = Budget::default();
        let mut state =
            json!({"history":[{"type":"message","role":"user","turnId":"one","content":[]}]});
        let mut r = b.reserve("chat", &state, true).unwrap();
        state["history"][0]["content"] =
            json!([{"type":"image"},{"type":"image"},{"type":"image"},{"type":"image"}]);
        assert!(r.sent(&state, true).is_err());
    }
    #[test]
    fn other_images_arriving_between_video_calls_reduce_the_allowance() {
        let b = Budget::default();
        let mut state =
            json!({"history":[{"type":"message","role":"user","turnId":"one","content":[]}]});
        for _ in 0..2 {
            let mut r = b.reserve("chat", &state, true).unwrap();
            r.sent(&state, true).unwrap();
        }
        state["history"][0]["content"] = json!([{"type":"image"},{"type":"image"}]);
        assert!(b.reserve("chat", &state, true).is_err());
    }
    #[test]
    fn image_turn_budget_and_concurrency_are_shared_with_prior_images() {
        let b = Budget::default();
        let state = json!({"history":[{"type":"message","role":"user","turnId":"one","content":[{"type":"image"}]}]});
        for _ in 0..vw_gate::MAX_IMAGES - 1 {
            let mut r = b.reserve("chat", &state, true).unwrap();
            assert!(b.reserve("chat", &state, false).is_err());
            r.sent(&state, true).unwrap();
        }
        assert!(b.reserve("chat", &state, true).is_err());
        let _one = b.reserve("one", &state, false).unwrap();
        let _two = b.reserve("two", &state, false).unwrap();
        assert!(b.reserve("three", &state, false).is_err());
    }
}
