fn main() -> eros::Result<()> {
    let runtime = compio::runtime::Runtime::new()?;
    runtime.block_on(rabbit::run())
}
