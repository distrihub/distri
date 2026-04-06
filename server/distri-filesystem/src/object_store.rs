use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use distri_types::configuration::ObjectStorageConfig;
use object_store::ObjectStore;

/// Build an object store instance from configuration
pub fn build_object_store(config: &ObjectStorageConfig) -> Result<Arc<dyn ObjectStore>> {
    match config {
        ObjectStorageConfig::FileSystem { base_path } => {
            std::fs::create_dir_all(base_path).with_context(|| {
                format!("failed to create filesystem object store at {}", base_path)
            })?;
            let store = object_store::local::LocalFileSystem::new_with_prefix(base_path)
                .with_context(|| {
                    format!("failed to build filesystem object store at {}", base_path)
                })?;
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
                .context("failed to build amazon s3 object store")?;
            Ok(Arc::new(store))
        }
        ObjectStorageConfig::GoogleCloudStorage { .. } => Err(anyhow!(
            "Google Cloud Storage object store not yet supported"
        )),
    }
}
