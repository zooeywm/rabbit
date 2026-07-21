#[derive(Debug)]
pub(crate) struct ClientVideoFrameProbe;

#[derive(Debug, Default)]
pub(crate) struct ClientVideoProbeReporter;

impl ClientVideoProbeReporter {
    pub(crate) fn record_frame(&mut self, _probe: ClientVideoFrameProbe) {}
}

// Focused test: cargo test infra::platform::client_video_probe::tests:: --lib
#[cfg(test)]
mod tests {
    use crate::infra::platform::client_video_probe::{
        ClientVideoFrameProbe, ClientVideoProbeReporter,
    };

    #[test]
    fn client_video_probe_accepts_a_completed_frame() {
        let mut reporter = ClientVideoProbeReporter;

        reporter.record_frame(ClientVideoFrameProbe);
    }
}
