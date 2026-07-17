#[derive(Debug)]
pub(crate) struct KmsCaptureWorker {
    screen_name: String,
}

impl KmsCaptureWorker {
    pub(crate) fn new(screen_name: String) -> Self {
        Self { screen_name }
    }
}

#[cfg(test)]
mod tests {
    use crate::infra::platform::screen_capture::kms::worker::KmsCaptureWorker;

    #[test]
    fn empty_worker_retains_screen_name() {
        let worker = KmsCaptureWorker::new("eDP-1".to_owned());

        assert_eq!(worker.screen_name, "eDP-1");
    }
}
