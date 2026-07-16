use crate::kernel::{
    screen_configuration::{ScreenStreamsConfigured, SetScreenStreams},
    screen_manager::{ScreenId, ScreenLayout},
};

#[derive(Debug, Clone, PartialEq)]
pub struct ScreenInfo {
    pub id: ScreenId,
    pub name: String,
    pub layout: ScreenLayout,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ControlMessage {
    ScreenList(Vec<ScreenInfo>),
    SetScreenStreams(SetScreenStreams),
    ScreenStreamsConfigured(ScreenStreamsConfigured),
}
