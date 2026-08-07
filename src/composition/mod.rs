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

pub(super) fn create_app()
-> impl FnOnce() -> eros::Result<selected_platform::PlatformApp> + Send + 'static {
    selected_platform::create_app()
}
