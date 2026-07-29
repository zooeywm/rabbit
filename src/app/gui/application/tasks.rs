use crate::{
    app::gui::application::message::{MessageSender, RootMessage},
    infra::{ConnectionEndpoint, receive_request},
    kernel::{
        session::{SessionMessage, SessionRecv},
        transport::TransportRecv,
    },
};

pub(super) async fn receive_connection_requests(
    endpoint: ConnectionEndpoint,
    sender: MessageSender,
) {
    loop {
        let connection = match endpoint.accept_connection().await {
            Ok(Some(connection)) => connection,
            Ok(None) => return,
            Err(error) => {
                sender.post(RootMessage::ConnectionListenerFailed(error));
                return;
            }
        };

        match receive_request(connection).await {
            Ok(Some(request)) => sender.post(RootMessage::ConnectionRequest(request)),
            Ok(None) => {}
            Err(error) => sender.post(RootMessage::ConnectionRequestFailed(error)),
        }
    }
}

pub(super) async fn receive_session<R>(mut session: SessionRecv<R>, sender: MessageSender)
where
    R: TransportRecv,
{
    let id = session.id();

    loop {
        match session.recv().await {
            Ok(Some(SessionMessage::Video(frame))) => {
                sender.post(RootMessage::VideoFrameReceived(id, frame));
            }
            Ok(Some(message)) => sender.post_session_message(id, message),
            Ok(None) => {
                sender.post(RootMessage::SessionClosed(id));
                return;
            }
            Err(error) => {
                sender.post(RootMessage::SessionFailed(id, error));
                return;
            }
        }
    }
}
