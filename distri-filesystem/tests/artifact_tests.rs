#[cfg(test)]
mod tests {
    use distri_filesystem::{ArtifactStorageConfig, ArtifactWrapper, FileSystemConfig};
    use distri_types::configuration::ObjectStorageConfig;
    use distri_types::{Part, ToolResponse};
    use serde_json::json;
    use tempfile::TempDir;

    async fn setup_test_env() -> (TempDir, distri_filesystem::FileSystem) {
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let config = FileSystemConfig {
            object_store: ObjectStorageConfig::FileSystem {
                base_path: temp_dir.path().to_string_lossy().to_string(),
            },
            root_prefix: Some("testrun".to_string()),
        };
        let filesystem = distri_filesystem::create_file_system(config).await.unwrap();
        (temp_dir, filesystem)
    }

    #[tokio::test]
    async fn test_comprehensive_artifact_operations() {
        let (temp_dir, filesystem) = setup_test_env().await;
        let thread_id = "test_thread_123";
        let task_id = "test_task_456";

        let base_path = ArtifactWrapper::task_namespace(thread_id, task_id);
        let wrapper =
            ArtifactWrapper::new(std::sync::Arc::new(filesystem.clone()), base_path.clone())
                .await
                .unwrap();
        let config = ArtifactStorageConfig::for_testing();

        println!("🧪 Testing comprehensive artifact operations");

        // Test JSON data
        let json1 = json!({
            "id": 1,
            "name": "Singapore Cabinet",
            "ministers": ["PM Lee", "DPM Wong", "Minister Ong"],
            "data": {
                "year": 2024,
                "positions": 20
            }
        });

        // Create tool response with large content that should be stored as artifacts
        let tool_response1 = ToolResponse {
            tool_call_id: "call1".to_string(),
            tool_name: "get_singapore_data".to_string(),
            parts: vec![Part::Data(json1.clone())],
        };

        // Process tool response - should convert large data to artifacts
        let processed1 = wrapper
            .process_tool_response(tool_response1, &config)
            .await
            .expect("Failed to process tool response 1");

        println!("✅ Processed tool response and converted to artifact");

        // Verify that large data was converted to artifact
        assert_eq!(processed1.parts.len(), 1);

        match &processed1.parts[0] {
            Part::Artifact(metadata) => {
                println!("✅ Response converted to artifact: {}", metadata.file_id);
                assert!(metadata.relative_path.contains(".json"));
                assert_eq!(metadata.content_type, Some("application/json".to_string()));
            }
            _ => panic!("Expected artifact, got {:?}", processed1.parts[0]),
        }

        // Test saving and reading artifacts directly
        wrapper
            .save_artifact(
                "test_singapore.json",
                &serde_json::to_string_pretty(&json1).unwrap(),
            )
            .await
            .expect("Failed to save artifact");

        println!("✅ Saved artifact directly");

        // Test listing artifacts
        let artifacts = wrapper
            .list_artifacts()
            .await
            .expect("Failed to list artifacts");
        println!("✅ Listed {} artifacts", artifacts.len());
        assert!(
            !artifacts.is_empty(),
            "artifact listing should contain saved files"
        );
        assert!(
            artifacts
                .iter()
                .any(|entry| entry.name == "test_singapore.json"),
            "artifact listing should include saved filename"
        );

        // Verify the file was written with the expected path (no percent-encoding)
        let expected_path = temp_dir
            .path()
            .join("testrun")
            .join(&base_path)
            .join("content")
            .join("test_singapore.json");
        assert!(
            expected_path.exists(),
            "expected artifact file to exist at {:?}",
            expected_path
        );
        assert!(
            !expected_path.to_string_lossy().contains("%2F"),
            "artifact path should not include percent-encoded separators: {:?}",
            expected_path
        );

        // Test reading artifact
        let content = wrapper
            .read_artifact("test_singapore.json", None, None)
            .await
            .expect("Failed to read artifact");

        println!("✅ Read artifact content length: {}", content.content.len());
        assert!(!content.content.is_empty());

        // Test searching artifacts
        let search_results = wrapper
            .search_artifacts("Singapore")
            .await
            .expect("Failed to search artifacts");

        println!("✅ Search found {} matches", search_results.matches.len());
        assert!(!search_results.matches.is_empty());

        // Test cleanup task folder
        wrapper
            .cleanup_task_folder()
            .await
            .expect("Failed to cleanup task folder");

        println!("✅ Cleaned up task folder successfully");

        println!("🎉 All comprehensive artifact operations completed successfully!");
    }

    #[tokio::test]
    async fn test_different_content_types() {
        let (_temp_dir, filesystem) = setup_test_env().await;
        let thread_id = "test_thread_456";
        let task_id = "test_task_789";

        let base_path = ArtifactWrapper::task_namespace(thread_id, task_id);
        let wrapper = ArtifactWrapper::new(std::sync::Arc::new(filesystem), base_path)
            .await
            .unwrap();
        let config = ArtifactStorageConfig::for_testing();

        println!("🧪 Testing different content types");

        // Test text content
        let text_response = ToolResponse {
            tool_call_id: "text_call".to_string(),
            tool_name: "generate_text".to_string(),
            parts: vec![Part::Text(
                "This is a large text content that should be stored as an artifact.".to_string(),
            )],
        };

        // Test JSON data
        let data_response = ToolResponse {
            tool_call_id: "data_call".to_string(),
            tool_name: "generate_data".to_string(),
            parts: vec![Part::Data(
                json!({"key": "value", "numbers": [1, 2, 3, 4, 5]}),
            )],
        };

        let processed_text = wrapper
            .process_tool_response(text_response, &config)
            .await
            .expect("Failed to process text response");

        let processed_data = wrapper
            .process_tool_response(data_response, &config)
            .await
            .expect("Failed to process data response");

        // Verify text was stored as artifact with .txt extension
        if let Part::Artifact(metadata) = &processed_text.parts[0] {
            assert!(metadata.relative_path.contains(".txt"));
            assert_eq!(metadata.content_type, Some("text/plain".to_string()));
            println!("✅ Text content stored with .txt extension");
        } else {
            panic!("Text should have been converted to artifact");
        }

        // Verify data was stored as artifact with .json extension
        if let Part::Artifact(metadata) = &processed_data.parts[0] {
            assert!(metadata.relative_path.contains(".json"));
            assert_eq!(metadata.content_type, Some("application/json".to_string()));
            println!("✅ JSON data stored with .json extension");
        } else {
            panic!("Data should have been converted to artifact");
        }

        println!("🎉 Different content types test completed successfully!");
    }

    #[tokio::test]
    async fn test_storage_config_thresholds() {
        let (_temp_dir, filesystem) = setup_test_env().await;
        let thread_id = "test_thread";
        let task_id = "test_task";

        println!("🧪 Testing storage configuration thresholds");

        let base_path = ArtifactWrapper::task_namespace(thread_id, task_id);
        let wrapper = ArtifactWrapper::new(std::sync::Arc::new(filesystem), base_path)
            .await
            .unwrap();

        // Test with production config (higher thresholds)
        let prod_config = ArtifactStorageConfig::for_production();

        // Small content that shouldn't be stored in production
        let small_response = ToolResponse {
            tool_call_id: "small_call".to_string(),
            tool_name: "small_data".to_string(),
            parts: vec![Part::Text("Small text".to_string())],
        };

        let processed_small = wrapper
            .process_tool_response(small_response, &prod_config)
            .await
            .expect("Failed to process small response");

        // Should remain as text, not converted to artifact
        match &processed_small.parts[0] {
            Part::Text(text) => {
                assert_eq!(text, "Small text");
                println!("✅ Small content not converted to artifact in production config");
            }
            _ => panic!("Small text should not be converted to artifact in production"),
        }

        // Test with testing config (always store)
        let test_config = ArtifactStorageConfig::for_testing();

        let same_small_response = ToolResponse {
            tool_call_id: "small_call2".to_string(),
            tool_name: "small_data".to_string(),
            parts: vec![Part::Text("Small text".to_string())],
        };

        let processed_test = wrapper
            .process_tool_response(same_small_response, &test_config)
            .await
            .expect("Failed to process small response in test config");

        // Should be converted to artifact in testing config
        match &processed_test.parts[0] {
            Part::Artifact(_) => {
                println!("✅ Small content converted to artifact in testing config");
            }
            _ => panic!("Small text should be converted to artifact in testing config"),
        }

        println!("🎉 Storage configuration thresholds test completed successfully!");
    }
}
