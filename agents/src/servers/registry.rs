use crate::types::ToolDefinition;
use anyhow::Result;
use mcp_sdk::{server::Server, transport::Transport};
use std::any::Any;
use std::collections::HashMap;

type DynServerBuilder =
    Box<dyn Fn(Box<dyn Any + Send>) -> Result<Box<dyn ServerTrait>> + Send + Sync>;

// This registry is only really for local running agents using async methos
pub struct ServerRegistry {
    builders: HashMap<String, DynServerBuilder>,
}

impl ServerRegistry {
    pub fn new() -> Self {
        Self {
            builders: HashMap::new(),
        }
    }

    pub fn register<T, F>(&mut self, name: String, builder: F)
    where
        T: Transport + 'static,
        F: Fn(T) -> Result<Server<T>> + Send + Sync + 'static,
    {
        let wrapped_builder = move |transport: Box<dyn Any + Send>| {
            let transport = *transport
                .downcast::<T>()
                .map_err(|_| anyhow::anyhow!("Transport type mismatch"))?;
            let server = builder(transport)?;
            Ok(Box::new(server) as Box<dyn ServerTrait>)
        };
        self.builders.insert(name, Box::new(wrapped_builder));
    }

    pub async fn run<T: Transport + 'static>(
        &self,
        tool_def: ToolDefinition,
        transport: T,
    ) -> Result<()> {
        let mcp_server = tool_def.mcp_server.to_string();

        match self.builders.get(&mcp_server) {
            Some(builder) => {
                let server = builder(Box::new(transport))?;
                server.listen().await
            }
            None => Err(anyhow::anyhow!("MCP Server: {} is not found", mcp_server)),
        }
    }
}

#[async_trait::async_trait]
pub trait ServerTrait: Send + Sync {
    async fn listen(&self) -> Result<()>;
}

#[async_trait::async_trait]
impl<T: Transport> ServerTrait for Server<T> {
    async fn listen(&self) -> Result<()> {
        self.listen().await
    }
}
