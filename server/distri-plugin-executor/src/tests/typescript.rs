use std::{path::Path, path::PathBuf, sync::Arc};

use distri_types::{MockOrchestrator, ToolCall};

use crate::{
    plugin_trait::{PluginContext, PluginExecutor, PluginLoadContext},
    TypeScriptPluginExecutor,
};
use anyhow::Context;
use distri_types::DistriServerConfig;

#[tokio::test]
async fn load_execute_plugin() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from(
            "distri_plugin_executor=debug,info",
        ))
        .try_init();

    let plugin = TypeScriptPluginExecutor::new(Arc::new(MockOrchestrator)).unwrap();
    let plugin_path = Path::new("./").join("samples/hello-plugin");

    let manifest = plugin_path.join("distri.toml");
    if !manifest.exists() {
        eprintln!("skipping typescript plugin test; samples/hello-plugin not found");
        return;
    }
    let configuration = DistriServerConfig::load_from_path(&manifest)
        .await
        .expect("failed to load distri.toml");

    let entrypoint = configuration
        .entrypoints
        .as_ref()
        .map(|entry| entry.path.clone())
        .expect("typescript entrypoint required for test plugin");

    struct FsResolver {
        root: PathBuf,
    }

    impl crate::plugin_trait::PluginFileResolver for FsResolver {
        fn read(&self, path: &str) -> anyhow::Result<Vec<u8>> {
            let sanitized = path.trim_start_matches('/');
            let full_path = self.root.join(sanitized);
            std::fs::read(&full_path)
                .with_context(|| format!("failed to read {}", full_path.display()))
        }
    }

    let context = PluginLoadContext {
        package_name: configuration.name.clone(),
        entrypoint: Some(entrypoint.clone()),
        manifest: configuration.clone(),
        resolver: Arc::new(FsResolver {
            root: plugin_path.clone(),
        }),
    };

    let plugin_name = plugin.load_plugin(context).await.unwrap();

    let plugin_info = plugin.get_plugin_info("hello-plugin").await.unwrap();

    assert_eq!(plugin_name, "hello-plugin");
    assert!(!plugin_info.integrations.is_empty());
    assert!(!plugin_info.integrations[0].tools.is_empty());

    let result = plugin
        .execute_tool(
            "hello-plugin",
            &ToolCall {
                tool_call_id: "123".to_string(),
                tool_name: "hello".to_string(),
                input: serde_json::json!({}),
            },
            PluginContext {
                call_id: "123".to_string(),
                agent_id: Some("123".to_string()),
                session_id: Some("123".to_string()),
                task_id: Some("123".to_string()),
                run_id: Some("123".to_string()),
                user_id: Some("user-123".to_string()),
                params: serde_json::json!({}),
                secrets: std::collections::HashMap::new(),
                auth_session: None,
            },
        )
        .await
        .unwrap();

    println!("result: {:?}", result);
}
