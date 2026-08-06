macro_rules! impl_capturer_boilerplate {
    ($manager_state:ident, $manager_impl:ident, $capturer_state:ident) => {
        impl $crate::app::container::root::outbound_port::CapturerManagerStateSpec
            for $manager_state
        {
            type ScreenCapturerState = $capturer_state;
        }

        impl<CvtMgrSt, EcdMgrSt> AsRef<$manager_state>
            for $crate::app::container::RootContainer<$manager_state, CvtMgrSt, EcdMgrSt>
        where
            CvtMgrSt:
                $crate::app::container::root::outbound_port::ConverterManagerStateSpec,
            EcdMgrSt: $crate::app::container::root::outbound_port::EncoderManagerStateSpec,
        {
            fn as_ref(&self) -> &$manager_state {
                self.capturer_manager_state()
            }
        }

        impl<CvtMgrSt, EcdMgrSt> AsMut<$manager_state>
            for $crate::app::container::RootContainer<$manager_state, CvtMgrSt, EcdMgrSt>
        where
            CvtMgrSt:
                $crate::app::container::root::outbound_port::ConverterManagerStateSpec,
            EcdMgrSt: $crate::app::container::root::outbound_port::EncoderManagerStateSpec,
        {
            fn as_mut(&mut self) -> &mut $manager_state {
                self.capturer_manager_state_mut()
            }
        }

        impl<CvtMgrSt, EcdMgrSt>
            $crate::app::container::root::outbound_port::CapturerManager
            for $crate::app::container::RootContainer<$manager_state, CvtMgrSt, EcdMgrSt>
        where
            CvtMgrSt:
                $crate::app::container::root::outbound_port::ConverterManagerStateSpec,
            EcdMgrSt: $crate::app::container::root::outbound_port::EncoderManagerStateSpec,
        {
            type ScreenCapturerState = $capturer_state;

            fn create_screen_capturer(
                &mut self,
                capture_source_id: $crate::domain::stream::models::vo::CaptureSourceId,
            ) -> eros::Result<Self::ScreenCapturerState> {
                $crate::app::container::root::outbound_port::CapturerManager::create_screen_capturer(
                    $manager_impl::inj_ref_mut(self),
                    capture_source_id,
                )
            }
        }

        impl<CvtSt, EcdSt> AsRef<$capturer_state>
            for $crate::app::container::CaptureSourceContainer<$capturer_state, CvtSt, EcdSt>
        {
            fn as_ref(&self) -> &$capturer_state {
                self.screen_capturer_state()
            }
        }

        impl<CvtSt, EcdSt> AsMut<$capturer_state>
            for $crate::app::container::CaptureSourceContainer<$capturer_state, CvtSt, EcdSt>
        {
            fn as_mut(&mut self) -> &mut $capturer_state {
                self.screen_capturer_state_mut()
            }
        }
    };
}

macro_rules! impl_converter_boilerplate {
    ($manager_state:ident, $manager_impl:ident, $converter_state:ident) => {
        impl $crate::app::container::root::outbound_port::ConverterManagerStateSpec
            for $manager_state
        {
            type EncoderFrameConverterState = $converter_state;
        }

        impl<CapMgrSt, EcdMgrSt> AsRef<$manager_state>
            for $crate::app::container::RootContainer<CapMgrSt, $manager_state, EcdMgrSt>
        where
            CapMgrSt:
                $crate::app::container::root::outbound_port::CapturerManagerStateSpec,
            EcdMgrSt: $crate::app::container::root::outbound_port::EncoderManagerStateSpec,
        {
            fn as_ref(&self) -> &$manager_state {
                self.converter_manager_state()
            }
        }

        impl<CapMgrSt, EcdMgrSt> AsMut<$manager_state>
            for $crate::app::container::RootContainer<CapMgrSt, $manager_state, EcdMgrSt>
        where
            CapMgrSt:
                $crate::app::container::root::outbound_port::CapturerManagerStateSpec,
            EcdMgrSt: $crate::app::container::root::outbound_port::EncoderManagerStateSpec,
        {
            fn as_mut(&mut self) -> &mut $manager_state {
                self.converter_manager_state_mut()
            }
        }

        impl<CapMgrSt, EcdMgrSt>
            $crate::app::container::root::outbound_port::ConverterManager
            for $crate::app::container::RootContainer<CapMgrSt, $manager_state, EcdMgrSt>
        where
            CapMgrSt:
                $crate::app::container::root::outbound_port::CapturerManagerStateSpec,
            EcdMgrSt: $crate::app::container::root::outbound_port::EncoderManagerStateSpec,
        {
            type EncoderFrameConverterState = $converter_state;

            fn create_encoder_frame_converter(
                &mut self,
            ) -> eros::Result<Self::EncoderFrameConverterState> {
                $crate::app::container::root::outbound_port::ConverterManager::create_encoder_frame_converter(
                    $manager_impl::inj_ref_mut(self),
                )
            }
        }

        impl<EcdSt> AsRef<$converter_state>
            for $crate::app::container::StreamPipelineContainer<$converter_state, EcdSt>
        {
            fn as_ref(&self) -> &$converter_state {
                self.encoder_frame_converter_state()
            }
        }

        impl<EcdSt> AsMut<$converter_state>
            for $crate::app::container::StreamPipelineContainer<$converter_state, EcdSt>
        {
            fn as_mut(&mut self) -> &mut $converter_state {
                self.encoder_frame_converter_state_mut()
            }
        }
    };
}

macro_rules! impl_encoder_boilerplate {
    ($manager_state:ident, $manager_impl:ident, $encoder_state:ident) => {
        impl $crate::app::container::root::outbound_port::EncoderManagerStateSpec
            for $manager_state
        {
            type VideoEncoderState = $encoder_state;
        }

        impl<CapMgrSt, CvtMgrSt> AsRef<$manager_state>
            for $crate::app::container::RootContainer<CapMgrSt, CvtMgrSt, $manager_state>
        where
            CapMgrSt: $crate::app::container::root::outbound_port::CapturerManagerStateSpec,
            CvtMgrSt: $crate::app::container::root::outbound_port::ConverterManagerStateSpec,
        {
            fn as_ref(&self) -> &$manager_state {
                self.encoder_manager_state()
            }
        }

        impl<CapMgrSt, CvtMgrSt> AsMut<$manager_state>
            for $crate::app::container::RootContainer<CapMgrSt, CvtMgrSt, $manager_state>
        where
            CapMgrSt: $crate::app::container::root::outbound_port::CapturerManagerStateSpec,
            CvtMgrSt: $crate::app::container::root::outbound_port::ConverterManagerStateSpec,
        {
            fn as_mut(&mut self) -> &mut $manager_state {
                self.encoder_manager_state_mut()
            }
        }

        impl<CapMgrSt, CvtMgrSt> $crate::app::container::root::outbound_port::EncoderManager
            for $crate::app::container::RootContainer<CapMgrSt, CvtMgrSt, $manager_state>
        where
            CapMgrSt: $crate::app::container::root::outbound_port::CapturerManagerStateSpec,
            CvtMgrSt: $crate::app::container::root::outbound_port::ConverterManagerStateSpec,
        {
            type VideoEncoderState = $encoder_state;

            fn create_video_encoder(&mut self) -> eros::Result<Self::VideoEncoderState> {
                $crate::app::container::root::outbound_port::EncoderManager::create_video_encoder(
                    $manager_impl::inj_ref_mut(self),
                )
            }
        }

        impl<CvtSt> AsRef<$encoder_state>
            for $crate::app::container::StreamPipelineContainer<CvtSt, $encoder_state>
        {
            fn as_ref(&self) -> &$encoder_state {
                self.video_encoder_state()
            }
        }

        impl<CvtSt> AsMut<$encoder_state>
            for $crate::app::container::StreamPipelineContainer<CvtSt, $encoder_state>
        {
            fn as_mut(&mut self) -> &mut $encoder_state {
                self.video_encoder_state_mut()
            }
        }
    };
}

macro_rules! impl_root_container {
    ($capturer_manager_state:ident, $converter_manager_state:ident, $encoder_manager_state:ident) => {
        impl
            $crate::app::container::RootContainer<
                $capturer_manager_state,
                $converter_manager_state,
                $encoder_manager_state,
            >
        {
            pub(crate) fn new() -> eros::Result<Self> {
                Ok(Self::from_manager_states(
                    $capturer_manager_state::new()?,
                    $converter_manager_state::new()?,
                    $encoder_manager_state::new()?,
                ))
            }
        }
    };
}

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
