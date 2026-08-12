use std::sync::Mutex;

struct LatestDecodedFrameSlotState<Frame> {
    frame: Option<Frame>,
    closed: bool,
}

pub(crate) struct LatestDecodedFrameSlot<Frame> {
    state: Mutex<LatestDecodedFrameSlotState<Frame>>,
}

impl<Frame> LatestDecodedFrameSlot<Frame> {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(LatestDecodedFrameSlotState {
                frame: None,
                closed: false,
            }),
        }
    }

    pub(crate) fn replace(&self, frame: Frame) -> bool {
        let replaced = {
            let mut state = self
                .state
                .lock()
                .expect("latest decoded frame slot mutex poisoned");
            if state.closed {
                return false;
            }
            state.frame.replace(frame)
        };
        drop(replaced);
        true
    }

    pub(crate) fn take_latest(&self) -> Option<Frame> {
        self.state
            .lock()
            .expect("latest decoded frame slot mutex poisoned")
            .frame
            .take()
    }

    pub(crate) fn close(&self) {
        let pending = {
            let mut state = self
                .state
                .lock()
                .expect("latest decoded frame slot mutex poisoned");
            if state.closed {
                return;
            }
            state.closed = true;
            state.frame.take()
        };
        drop(pending);
    }
}

#[cfg(test)]
mod tests {
    use super::LatestDecodedFrameSlot;

    #[test]
    fn latest_decoded_frame_replaces_an_older_frame() {
        let slot = LatestDecodedFrameSlot::new();
        assert!(slot.replace(1));
        assert!(slot.replace(2));
        assert_eq!(slot.take_latest(), Some(2));
    }
}
