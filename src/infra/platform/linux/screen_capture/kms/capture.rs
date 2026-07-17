pub(crate) fn capture_one_frame() -> eros::Result<Option<()>> {
    Ok(None)
}

#[cfg(test)]
mod tests {
    use crate::infra::platform::screen_capture::kms::capture::capture_one_frame;

    #[test]
    fn empty_capture_runs() {
        let frame = capture_one_frame().expect("KMS capture should run");

        assert!(frame.is_none());
    }
}
