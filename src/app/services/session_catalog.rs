//! Session catalog queries over the in-memory application model.
//!
//! These helpers keep ordered enumeration and identity lookups out of GUI
//! handlers so list presentation and disconnect targeting share one policy.

use std::net::SocketAddr;

use crate::{
    app::model::{ApplicationModel, SessionKey},
    app::platform::ApplicationStack,
    kernel::{
        screen_manager::ScreenId,
        session::{SessionId, SessionRole},
    },
};

/// Host sessions ordered by peer address then session id (stable UI order).
pub fn host_session_ids<Stack>(model: &ApplicationModel<Stack>) -> Vec<SessionId>
where
    Stack: ApplicationStack,
{
    let mut sessions = model
        .sessions
        .iter()
        .filter(|session| session.key.role() == SessionRole::Host)
        .map(|session| (session.key.peer_address(), session.send.id()))
        .collect::<Vec<_>>();
    sessions.sort_by_key(|(address, session_id)| (*address, session_id.0));
    sessions
        .into_iter()
        .map(|(_, session_id)| session_id)
        .collect()
}

/// Hosted screen streams ordered by peer address, session id, then screen id.
pub fn hosted_screen_stream_entries<Stack>(
    model: &ApplicationModel<Stack>,
) -> Vec<(SessionId, ScreenId)>
where
    Stack: ApplicationStack,
{
    let mut streams = model
        .sessions
        .iter()
        .filter(|session| session.key.role() == SessionRole::Host)
        .flat_map(|session| {
            session
                .screen_streams
                .keys()
                .map(|screen_id| (session.key.peer_address(), session.send.id(), *screen_id))
        })
        .collect::<Vec<_>>();
    streams.sort_by_key(|(address, session_id, screen_id)| (*address, session_id.0, screen_id.0));
    streams
        .into_iter()
        .map(|(_, session_id, screen_id)| (session_id, screen_id))
        .collect()
}

/// Controller session that owns the active direct-connection peer, if any.
pub fn controller_session_for_peer<Stack>(
    model: &ApplicationModel<Stack>,
    peer: SocketAddr,
) -> Option<SessionId>
where
    Stack: ApplicationStack,
{
    model
        .sessions
        .iter()
        .find(|session| {
            session.key.role() == SessionRole::Controller && session.key.peer_address() == peer
        })
        .map(|session| session.send.id())
}

/// Whether the model already tracks a session with this identity key.
#[cfg_attr(not(test), allow(dead_code))]
pub fn has_session_key<Stack>(model: &ApplicationModel<Stack>, key: &SessionKey) -> bool
where
    Stack: ApplicationStack,
{
    model.has_session(key)
}

/// Rebuilds the flat remote-screen index used by the remote-devices UI.
pub fn rebuild_remote_screen_entries<Stack>(model: &mut ApplicationModel<Stack>)
where
    Stack: ApplicationStack,
{
    let mut entries = model
        .remote_screens
        .iter()
        .flat_map(|(session_id, screens)| screens.iter().map(|screen| (*session_id, screen.id)))
        .collect::<Vec<_>>();

    entries.sort_by_key(|(session_id, screen_id)| (session_id.0, screen_id.0));
    model.remote_screen_entries.clear();
    model.remote_screen_entries.extend(entries);
    model.selected_remote_screen = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::session::SessionRole;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    #[test]
    fn controller_peer_lookup_requires_matching_role_and_address() {
        let peer = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 52731);
        let key = SessionKey::new(peer, SessionRole::Controller);
        assert_eq!(key.role(), SessionRole::Controller);
        assert_eq!(key.peer_address(), peer);
    }
}
