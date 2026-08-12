pub(crate) trait ClientApplication {
    async fn shutdown(self) -> eros::Result<()>;
}

impl<DcdMgrSt> ClientApplication for super::ClientContainer<DcdMgrSt> {
    async fn shutdown(self) -> eros::Result<()> {
        Ok(())
    }
}
