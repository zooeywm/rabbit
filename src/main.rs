fn main() -> eros::Result<()> {
    let test_ui = std::env::args().skip(1).any(|arg| arg == "--test");

    if test_ui {
        #[cfg(feature = "test-ui")]
        return rabbit::RabbitApp::new().run_test_ui();

        #[cfg(not(feature = "test-ui"))]
        eros::bail!("Test UI requires the `test-ui` feature");
    }

    rabbit::RabbitApp::new().run()
}
