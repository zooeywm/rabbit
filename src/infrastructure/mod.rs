pub(crate) mod support;

cfg_if::cfg_if! {
    if #[cfg(feature = "fake")] {
        #[path = "platform/fake/mod.rs"]
        mod fake;
        use fake as selected_platform;
    } else if #[cfg(target_os = "linux")] {
        #[path = "platform/linux/mod.rs"]
        mod linux;
        use linux as selected_platform;
    } else {
        #[path = "platform/unsupported/mod.rs"]
        mod unsupported;
        use unsupported as selected_platform;
    }
}

pub(crate) mod platform {
    pub(crate) use super::selected_platform::*;
}
