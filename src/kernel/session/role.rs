//! Session role authorization for control and media operations.

use crate::kernel::{session::SessionRole, session_control::ControlMessage};

pub(super) fn require_role(
    role: SessionRole,
    expected: SessionRole,
    operation: &str,
) -> eros::Result<()> {
    if role != expected {
        eros::bail!(
            "Session role {:?} cannot {operation}; expected {:?}",
            role,
            expected
        );
    }

    Ok(())
}

pub(super) fn validate_received_control(
    role: SessionRole,
    message: &ControlMessage,
) -> eros::Result<()> {
    let (expected, name) = match message {
        ControlMessage::ScreenList(_) => (SessionRole::Controller, "ScreenList"),
        ControlMessage::SetScreenStreams(_) => (SessionRole::Host, "SetScreenStreams"),
        ControlMessage::ScreenStreamsConfigured(_) => {
            (SessionRole::Controller, "ScreenStreamsConfigured")
        }
        ControlMessage::StopScreenStream(_) => return Ok(()),
        ControlMessage::RequestKeyFrame(_) => (SessionRole::Host, "RequestKeyFrame"),
        ControlMessage::RemoteInput(_) => (SessionRole::Host, "RemoteInput"),
    };

    if role != expected {
        eros::bail!(
            "Session role {:?} cannot receive {name}; expected {:?}",
            role,
            expected
        );
    }

    Ok(())
}
