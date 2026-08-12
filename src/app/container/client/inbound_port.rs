pub(crate) trait ClientApplication {
    async fn shutdown(self) -> eros::Result<()>;
}

impl ClientApplication for super::ClientContainer {
    async fn shutdown(self) -> eros::Result<()> {
        Ok(())
    }
}
