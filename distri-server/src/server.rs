use anyhow::Result;
use distri::servers::registry::Registry;

pub struct A2AServer {
    registry: Registry,
}

impl A2AServer {
    pub fn new(registry: Registry) -> Self {
        Self { registry }
    }

    pub async fn start(&self) -> Result<()> {
        // TODO: Implement A2A server methods
        Ok(())
    }
}
