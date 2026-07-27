fn main() -> eros::Result<()> {
    let headless = std::env::args().any(|arg| arg == "--headless" || arg == "-H");
    if headless {
        rabbit::run_headless()
    } else {
        rabbit::run()
    }
}
