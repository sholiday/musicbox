//! Lightweight in-memory metrics used by the CLI and debug HTTP server.
//!
//! We intentionally keep this module dumb and lock-based instead of
//! introducing an async runtime or channel fan-out. The controller loop is
//! synchronous and invoked from a single thread, so a small `RwLock` keeps the
//! implementation easy to reason about while still allowing the debug server
//! to read a snapshot without blocking the event loop for long.

use crate::controller::ControllerAction;
use std::sync::{Arc, RwLock};
use std::time::SystemTime;

#[derive(Debug, Clone, Default)]
pub struct StatusSnapshot {
    pub last_action: Option<ControllerAction>,
    pub last_update: Option<SystemTime>,
    pub idle_events: u64,
}

#[derive(Clone, Default)]
pub struct SharedStatus {
    inner: Arc<RwLock<StatusSnapshot>>,
}

impl SharedStatus {
    /// Record the latest controller action so UIs can see what triggered
    /// playback changes. We store the clone rather than the original reference
    /// because Axum serves JSON snapshots without holding the controller lock.
    pub fn record_action(&self, action: ControllerAction) {
        let mut guard = self.inner.write().expect("status write lock");
        guard.last_update = Some(SystemTime::now());
        guard.last_action = Some(action);
    }

    /// Count idle polls so we can detect when the reader is connected but no
    /// cards are present. This is helpful for debugging hardware issues.
    pub fn record_idle(&self) {
        let mut guard = self.inner.write().expect("status write lock");
        guard.last_update = Some(SystemTime::now());
        guard.idle_events += 1;
    }

    pub fn snapshot(&self) -> StatusSnapshot {
        self.inner.read().expect("status read lock").clone()
    }
}

pub fn init_logging() {
    use tracing_subscriber::{EnvFilter, fmt};

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let _ = fmt().with_env_filter(env_filter).try_init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_actions_and_idle_counts() {
        let status = SharedStatus::default();
        status.record_idle();

        let mut snapshot = status.snapshot();
        assert_eq!(snapshot.idle_events, 1);
        assert!(snapshot.last_action.is_none());
        assert!(snapshot.last_update.is_some());

        let action = ControllerAction::Started {
            card: crate::controller::CardUid::new(vec![1, 2, 3, 4]),
            track: crate::controller::Track::new("song.mp3".into()),
        };
        status.record_action(action.clone());

        snapshot = status.snapshot();
        assert_eq!(snapshot.idle_events, 1);
        assert_eq!(snapshot.last_action, Some(action));
        assert!(snapshot.last_update.is_some());
    }
}
