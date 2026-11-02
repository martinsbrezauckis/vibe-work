//! Paired wall/monotonic readings falsify clock-step bugs in Git throttles and install-offer lifetimes.
//! Uses the real cache/offer gates with Tokio time; no forge, vault or network is needed.
use super::ClockReading;
use crate::git_cmd::{ApiPause, CardCache};
use crate::install_offers::{OFFER_EXPIRED, OfferKey, Offers, STALE_OFFER, ToolsInstallOffer};
use std::time::Duration;

const WALL: u64 = 1_800_000_000_000;

fn reading(wall_ms: u64) -> ClockReading {
    ClockReading {
        wall_ms,
        instant: tokio::time::Instant::now(),
    }
}
fn stepped(forward: bool) -> u64 {
    if forward {
        WALL + 121_000
    } else {
        WALL - 119_000
    }
}

#[tokio::test(start_paused = true)]
async fn pr_refresh_floor_ignores_wall_steps() {
    let mut windows = Vec::new();
    for forward in [true, false] {
        let cache = CardCache {
            fetched_at_ms: WALL,
            fetched_clock: Some(reading(WALL)),
            ..CardCache::default()
        };
        tokio::time::advance(Duration::from_secs(1)).await;
        let fresh = cache.within_floor(reading(stepped(forward)), 30_000);
        tokio::time::advance(Duration::from_secs(30)).await;
        let expired = !cache.within_floor(reading(stepped(forward) + 30_000), 30_000);
        windows.push((fresh, expired));
        assert_eq!(
            cache.fetched_at_ms, WALL,
            "the visible fetch date stays an epoch point"
        );
    }
    assert_eq!(windows, [(true, true), (true, true)]);
}

#[tokio::test(start_paused = true)]
async fn api_rate_pause_ignores_wall_steps() {
    let mut remaining = Vec::new();
    for reset in [Some((WALL + 120_000) / 1000), None] {
        for forward in [true, false] {
            let mut pause = ApiPause::default();
            pause.set(reading(WALL), reset);
            let wait = if reset.is_some() { 120_000 } else { 3_600_000 };
            tokio::time::advance(Duration::from_secs(1)).await;
            remaining.push((pause.left(reading(stepped(forward))), wait - 1_000));
            tokio::time::advance(Duration::from_millis(wait)).await;
            remaining.push((pause.left(reading(stepped(forward) + wait)), 0));
        }
    }
    assert!(
        remaining
            .iter()
            .all(|(actual, expected)| actual == expected),
        "actual / expected remaining milliseconds: {remaining:?}"
    );
}

fn offer() -> ToolsInstallOffer {
    ToolsInstallOffer {
        offer_id: String::new(),
        kind: "skill".into(),
        name: "Example".into(),
        publisher: None,
        source: "local fixture".into(),
        licence: None,
        runs: "Instructions".into(),
        version: Some(1),
        expires_at: 0,
    }
}

#[tokio::test(start_paused = true)]
async fn install_offer_lifetime_ignores_wall_steps() {
    let mut windows = Vec::new();
    for forward in [true, false] {
        let offers = Offers::default();
        let key = OfferKey::new("install", "example", "");
        let first = offers.make(key.clone(), offer(), reading(WALL)).unwrap();
        let second = offers.make(key.clone(), offer(), reading(WALL)).unwrap();
        assert_eq!(
            first.expires_at,
            WALL + 600_000,
            "the offered expiry remains an epoch point"
        );
        tokio::time::advance(Duration::from_secs(1)).await;
        let wall = if forward {
            WALL + 3_601_000
        } else {
            WALL - 3_599_000
        };
        let fresh = offers.take(&first.offer_id, &key, reading(wall)).is_ok();
        tokio::time::advance(Duration::from_secs(600)).await;
        let expired = offers
            .take(&second.offer_id, &key, reading(wall + 600_000))
            .is_err_and(|e| e.server_code.as_deref() == Some(OFFER_EXPIRED));
        windows.push((fresh, expired));
    }
    assert_eq!(windows, [(true, true), (true, true)]);
}

#[tokio::test(start_paused = true)]
async fn offer_capacity_evicts_the_oldest_across_wall_steps() {
    for forward in [true, false] {
        let offers = Offers::default();
        let key = OfferKey::new("install", "example", "");
        let first = offers.make(key.clone(), offer(), reading(WALL)).unwrap();
        let mut last = first.clone();
        for n in 1..=64 {
            tokio::time::advance(Duration::from_millis(1)).await;
            last = offers
                .make(key.clone(), offer(), reading(stepped(forward) + n))
                .unwrap();
        }
        assert!(
            offers
                .take(&first.offer_id, &key, reading(stepped(forward) + 65))
                .is_err_and(|e| e.server_code.as_deref() == Some(STALE_OFFER)),
            "the oldest offer must leave first, including after a backward correction"
        );
        assert!(
            offers
                .take(&last.offer_id, &key, reading(stepped(forward) + 65))
                .is_ok()
        );
    }
}
