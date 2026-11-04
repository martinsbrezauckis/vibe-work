//! In-memory RPC channel attachment for core integration tests; uses normal routing and pump feeds.
use super::{ChannelKey, Channels, Connection, Feed};

impl Channels {
    /// Attach an in-memory RPC peer to the ordinary routing and pump in core tests.
    pub(crate) fn bind_test_peer(&self, id: &str, conn: Connection, key: ChannelKey) {
        let sub = conn.subscribe();
        self.bind(id, conn, key);
        let _ = self.feeds.send(Feed { key, sub });
    }
}
