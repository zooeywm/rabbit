mod inbound;

pub(crate) mod outbound_port;

use std::collections::HashMap;

use crate::{app::container::CaptureSourceContainer, domain::stream::models::vo::CaptureSourceId};

use self::outbound_port::{
    CapturerManagerStateSpec, ConverterManagerStateSpec, EncoderManagerStateSpec,
};

type CaptureSourceFor<CapMgrSt, CvtMgrSt, EcdMgrSt> = CaptureSourceContainer<
    <CapMgrSt as CapturerManagerStateSpec>::ScreenCapturerState,
    <CvtMgrSt as ConverterManagerStateSpec>::EncoderFrameConverterState,
    <EcdMgrSt as EncoderManagerStateSpec>::VideoEncoderState,
>;

pub(crate) struct RootContainer<CapMgrSt, CvtMgrSt, EcdMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
    CvtMgrSt: ConverterManagerStateSpec,
    EcdMgrSt: EncoderManagerStateSpec,
{
    /// Creates screen capturer states for capture-source containers.
    pub(super) capturer_manager_state: CapMgrSt,

    /// Creates frame-converter states for stream-pipeline containers.
    pub(super) converter_manager_state: CvtMgrSt,

    /// Creates video-encoder states for stream-pipeline containers.
    pub(super) encoder_manager_state: EcdMgrSt,

    /// Owns all active capture-source containers.
    pub(super) capture_sources:
        HashMap<CaptureSourceId, CaptureSourceFor<CapMgrSt, CvtMgrSt, EcdMgrSt>>,

    /// The numeric value assigned to the next successfully created stream.
    pub(super) next_stream_id: u16,
}
