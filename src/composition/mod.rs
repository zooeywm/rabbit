cfg_if::cfg_if! {
    if #[cfg(feature = "fake")] {
        #[path = "platform/fake.rs"]
        mod selected_platform;
    } else if #[cfg(target_os = "linux")] {
        #[path = "platform/linux.rs"]
        mod selected_platform;
    } else {
        #[path = "platform/unsupported.rs"]
        mod selected_platform;
    }
}

pub(super) fn compose_containers(
    app_message_sender: flume::Sender<crate::app::AppMessage>,
) -> eros::Result<selected_platform::PlatformContainers> {
    selected_platform::compose_containers(app_message_sender)
}
