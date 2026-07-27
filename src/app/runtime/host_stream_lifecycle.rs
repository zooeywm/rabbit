//! Pure host stream bookkeeping shared by GUI and headless shells.

use std::collections::HashMap;

use crate::{app::model::RunningScreenStream, kernel::screen_manager::ScreenId};

/// Outcome of applying a finished host screen-stream task to session state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenStreamFinishEffect {
    /// Stream id no longer matches (already replaced or removed).
    Stale,
    /// Current stream removed from the session map.
    Removed {
        /// Task returned an error and the session was not closed normally.
        failed: bool,
    },
}

/// Removes the current stream if `stream_id` still owns `screen_id`.
pub fn apply_screen_stream_finished(
    streams: &mut HashMap<ScreenId, RunningScreenStream>,
    screen_id: ScreenId,
    stream_id: u64,
    result_is_err: bool,
    session_closed_normally: bool,
) -> ScreenStreamFinishEffect {
    let is_current = streams
        .get(&screen_id)
        .is_some_and(|stream| stream.id == stream_id);
    if !is_current {
        return ScreenStreamFinishEffect::Stale;
    }
    streams.remove(&screen_id);
    ScreenStreamFinishEffect::Removed {
        failed: result_is_err && !session_closed_normally,
    }
}

/// Drops a host stream when the peer (or local UI) stops it. Returns whether a
/// stream was present.
pub fn apply_host_stop_screen_stream(
    streams: &mut HashMap<ScreenId, RunningScreenStream>,
    screen_id: ScreenId,
) -> bool {
    streams.remove(&screen_id).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::unsync_queue::UnsyncQueue;

    fn stream(id: u64) -> RunningScreenStream {
        RunningScreenStream {
            id,
            cancellation: UnsyncQueue::default(),
            encoder_commands: UnsyncQueue::default(),
            task: None,
        }
    }

    #[test]
    fn stale_finish_is_ignored() {
        let mut streams = HashMap::new();
        streams.insert(ScreenId(1), stream(10));
        let effect = apply_screen_stream_finished(&mut streams, ScreenId(1), 99, true, false);
        assert_eq!(effect, ScreenStreamFinishEffect::Stale);
        assert!(streams.contains_key(&ScreenId(1)));
    }

    #[test]
    fn current_finish_removes_and_classifies_failure() {
        let mut streams = HashMap::new();
        streams.insert(ScreenId(2), stream(7));
        let effect = apply_screen_stream_finished(&mut streams, ScreenId(2), 7, true, false);
        assert_eq!(effect, ScreenStreamFinishEffect::Removed { failed: true });
        assert!(streams.is_empty());

        streams.insert(ScreenId(2), stream(8));
        let effect = apply_screen_stream_finished(&mut streams, ScreenId(2), 8, true, true);
        assert_eq!(effect, ScreenStreamFinishEffect::Removed { failed: false });
    }

    #[test]
    fn host_stop_removes_stream() {
        let mut streams = HashMap::new();
        streams.insert(ScreenId(3), stream(1));
        assert!(apply_host_stop_screen_stream(&mut streams, ScreenId(3)));
        assert!(!apply_host_stop_screen_stream(&mut streams, ScreenId(3)));
    }
}
