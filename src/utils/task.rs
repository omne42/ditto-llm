pub struct AbortOnDrop(tokio::task::AbortHandle);

impl AbortOnDrop {
    pub fn new(handle: tokio::task::AbortHandle) -> Self {
        Self(handle)
    }
}

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}
