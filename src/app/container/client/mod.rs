pub(crate) mod inbound_port;
pub(crate) mod outbound_port;

pub(crate) struct ClientContainer<DcdMgrSt> {
    decoder_manager_state: DcdMgrSt,
}

impl<DcdMgrSt> ClientContainer<DcdMgrSt> {
    pub(crate) fn new(decoder_manager_state: DcdMgrSt) -> Self {
        Self {
            decoder_manager_state,
        }
    }

    pub(crate) fn decoder_manager_state(&self) -> &DcdMgrSt {
        &self.decoder_manager_state
    }

    pub(crate) fn decoder_manager_state_mut(&mut self) -> &mut DcdMgrSt {
        &mut self.decoder_manager_state
    }
}
