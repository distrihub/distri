use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use distri_plugin_executor::plugin_trait::PluginFileResolver;
use distri_types::configuration::ObjectStorageConfig;
use object_store::path::Path;
use object_store::ObjectStore;
use tokio::runtime::{Handle, RuntimeFlavor};
use tokio::task;

pub fn build_object_store(config: &ObjectStorageConfig) -> Result<Arc<dyn ObjectStore>> {
    match config {
        ObjectStorageConfig::FileSystem { base_path } => {
            std::fs::create_dir_all(base_path)
                .with_context(|| format!("Failed to create plugin directory at {}", base_path))?;
            let store = object_store::local::LocalFileSystem::new_with_prefix(base_path)
                .with_context(|| format!("Failed to create filesystem store at {}", base_path))?;
            Ok(Arc::new(store))
        }
        ObjectStorageConfig::S3 {
            bucket,
            region,
            endpoint,
            access_key_id,
            secret_access_key,
            path_style: _,
        } => {
            let mut builder = object_store::aws::AmazonS3Builder::new()
                .with_bucket_name(bucket)
                .with_region(region)
                .with_access_key_id(access_key_id)
                .with_secret_access_key(secret_access_key);

            if let Some(endpoint) = endpoint {
                builder = builder.with_endpoint(endpoint);
            }

            let store = builder
                .build()
                .context("Failed to create Amazon S3 object store")?;
            Ok(Arc::new(store))
        }
        ObjectStorageConfig::GoogleCloudStorage { .. } => Err(anyhow!(
            "Google Cloud Storage plugin store not yet supported"
        )),
    }
}

pub struct ObjectStorePluginResolver {
    store: Arc<dyn ObjectStore>,
    prefix: Path,
}

impl ObjectStorePluginResolver {
    pub fn new(store: Arc<dyn ObjectStore>, prefix: String) -> Result<Self> {
        let prefix_path = Path::from(prefix);
        Ok(Self {
            store,
            prefix: prefix_path,
        })
    }
}

impl PluginFileResolver for ObjectStorePluginResolver {
    fn read(&self, path: &str) -> Result<Vec<u8>> {
        let sanitized = path.trim_start_matches('/');
        let mut object_path = self.prefix.clone();
        for segment in sanitized.split('/') {
            if !segment.is_empty() {
                object_path = object_path.child(segment);
            }
        }

        let fetch = |store: Arc<dyn ObjectStore>, path: object_store::path::Path| async move {
            let get = store
                .get(&path)
                .await
                .with_context(|| format!("Failed to fetch plugin object at {}", path))?;
            let bytes = get
                .bytes()
                .await
                .context("Failed to read plugin object bytes")?;
            Ok::<Vec<u8>, anyhow::Error>(bytes.to_vec())
        };

        match Handle::try_current() {
            Ok(handle) => match handle.runtime_flavor() {
                RuntimeFlavor::MultiThread => {
                    let store = self.store.clone();
                    let path = object_path.clone();
                    let result = task::block_in_place(|| handle.block_on(fetch(store, path)))?;
                    Ok(result)
                }
                RuntimeFlavor::CurrentThread => {
                    let store = self.store.clone();
                    let path = object_path.clone();
                    let result = std::thread::spawn(move || -> Result<Vec<u8>> {
                        let runtime = tokio::runtime::Runtime::new()
                            .context("Failed to create Tokio runtime for plugin fetch")?;
                        runtime.block_on(fetch(store, path))
                    })
                    .join()
                    .map_err(|_| anyhow!("Plugin fetch thread panicked"))??;
                    Ok(result)
                }
                _ => {
                    let store = self.store.clone();
                    let path = object_path.clone();
                    let result = task::block_in_place(|| handle.block_on(fetch(store, path)))?;
                    Ok(result)
                }
            },
            Err(_) => {
                let store = self.store.clone();
                let path = object_path.clone();
                let result = std::thread::spawn(move || -> Result<Vec<u8>> {
                    let runtime = tokio::runtime::Runtime::new()
                        .context("Failed to create Tokio runtime for plugin fetch")?;
                    runtime.block_on(fetch(store, path))
                })
                .join()
                .map_err(|_| anyhow!("Plugin fetch thread panicked"))??;
                Ok(result)
            }
        }
    }
}
