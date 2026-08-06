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
    capturer_manager_state: CapMgrSt,

    converter_manager_state: CvtMgrSt,

    encoder_manager_state: EcdMgrSt,

    capture_sources: HashMap<CaptureSourceId, CaptureSourceFor<CapMgrSt, CvtMgrSt, EcdMgrSt>>,

    next_stream_id: u16,
}

impl<CapMgrSt, CvtMgrSt, EcdMgrSt> RootContainer<CapMgrSt, CvtMgrSt, EcdMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
    CvtMgrSt: ConverterManagerStateSpec,
    EcdMgrSt: EncoderManagerStateSpec,
{
    pub(crate) fn new(
        capturer_manager_state: CapMgrSt,
        converter_manager_state: CvtMgrSt,
        encoder_manager_state: EcdMgrSt,
    ) -> Self {
        Self {
            capturer_manager_state,
            converter_manager_state,
            encoder_manager_state,
            capture_sources: HashMap::new(),
            next_stream_id: 0,
        }
    }

    pub(crate) fn capturer_manager_state(&self) -> &CapMgrSt {
        &self.capturer_manager_state
    }

    pub(crate) fn capturer_manager_state_mut(&mut self) -> &mut CapMgrSt {
        &mut self.capturer_manager_state
    }

    pub(crate) fn converter_manager_state(&self) -> &CvtMgrSt {
        &self.converter_manager_state
    }

    pub(crate) fn converter_manager_state_mut(&mut self) -> &mut CvtMgrSt {
        &mut self.converter_manager_state
    }

    pub(crate) fn encoder_manager_state(&self) -> &EcdMgrSt {
        &self.encoder_manager_state
    }

    pub(crate) fn encoder_manager_state_mut(&mut self) -> &mut EcdMgrSt {
        &mut self.encoder_manager_state
    }
}
