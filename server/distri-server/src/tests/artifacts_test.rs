#[cfg(test)]
mod tests {
    use distri_filesystem::ArtifactWrapper;
    use distri_types::filesystem::FileSystemOps;
    use std::sync::Arc;
    use tempfile::TempDir;

    /// Helper to create a test filesystem with a temporary directory
    async fn create_test_filesystem() -> (Arc<dyn FileSystemOps>, TempDir) {
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let base_path = temp_dir.path().to_string_lossy().to_string();

        use distri_filesystem::{create_file_system, FileSystemConfig};
        use distri_types::configuration::ObjectStorageConfig;

        let config = FileSystemConfig {
            object_store: ObjectStorageConfig::FileSystem { base_path },
            root_prefix: None,
        };

        let filesystem = create_file_system(config)
            .await
            .expect("Failed to create filesystem");

        (Arc::new(filesystem), temp_dir)
    }

    #[tokio::test]
    async fn test_list_artifacts_checks_both_thread_and_task_levels() {
        // Create test filesystem
        let (filesystem, _temp_dir) = create_test_filesystem().await;

        // Use ArtifactNamespace to create paths consistently
        let thread_uuid = "29c53413-ad8d-40d5-a055-c4c9ac68e611";
        let task_uuid = "5ab25b3a-f2db-4975-a9d2-f5541dfb718c";

        let thread_namespace = distri_types::ArtifactNamespace::new(thread_uuid.to_string(), None);
        let task_namespace = distri_types::ArtifactNamespace::new(
            thread_uuid.to_string(),
            Some(task_uuid.to_string()),
        );

        let thread_path = thread_namespace.thread_path();
        let task_path = task_namespace.task_path().expect("Task path should exist");

        // Create artifacts in thread-level path
        let thread_wrapper = ArtifactWrapper::new(filesystem.clone(), thread_path.clone())
            .await
            .expect("Failed to create thread wrapper");

        thread_wrapper
            .save_artifact(
                "thread_artifact.json",
                r#"{"type": "thread", "data": "from_thread"}"#,
            )
            .await
            .expect("Failed to save thread artifact");

        // Create artifacts in task-level path
        let task_wrapper = ArtifactWrapper::new(filesystem.clone(), task_path.clone())
            .await
            .expect("Failed to create task wrapper");

        task_wrapper
            .save_artifact(
                "task_artifact.json",
                r#"{"type": "task", "data": "from_task"}"#,
            )
            .await
            .expect("Failed to save task artifact");

        // Test that list_artifacts checks both paths using ArtifactNamespace
        let mut all_artifacts = Vec::new();
        let mut seen_filenames = std::collections::HashSet::new();

        // Use all_paths() to get both thread and task paths
        let paths_to_check = task_namespace.all_paths();
        assert_eq!(
            paths_to_check.len(),
            2,
            "Should have both thread and task paths"
        );

        // Check each path
        for path_id in paths_to_check {
            let wrapper = ArtifactWrapper::new(filesystem.clone(), path_id.clone())
                .await
                .expect("Failed to create wrapper");

            match wrapper.list_artifacts().await {
                Ok(entries) => {
                    for e in entries {
                        if !seen_filenames.contains(&e.name) {
                            seen_filenames.insert(e.name.clone());
                            all_artifacts.push(e.name.clone());
                        }
                    }
                }
                Err(_) => {}
            }
        }

        // Verify both artifacts are found
        assert_eq!(
            all_artifacts.len(),
            2,
            "Should find artifacts from both thread and task levels"
        );
        assert!(
            all_artifacts.contains(&"thread_artifact.json".to_string()),
            "Should find thread artifact"
        );
        assert!(
            all_artifacts.contains(&"task_artifact.json".to_string()),
            "Should find task artifact"
        );
    }

    #[tokio::test]
    async fn test_list_artifacts_handles_duplicate_filenames() {
        // Test that if the same filename exists in both locations, we only return it once
        let (filesystem, _temp_dir) = create_test_filesystem().await;

        let thread_uuid = "29c53413-ad8d-40d5-a055-c4c9ac68e611";
        let task_uuid = "5ab25b3a-f2db-4975-a9d2-f5541dfb718c";
        let duplicate_filename = "duplicate.json";

        // Use ArtifactNamespace to create paths consistently
        let thread_namespace = distri_types::ArtifactNamespace::new(thread_uuid.to_string(), None);
        let task_namespace = distri_types::ArtifactNamespace::new(
            thread_uuid.to_string(),
            Some(task_uuid.to_string()),
        );

        let thread_path = thread_namespace.thread_path();
        let task_path = task_namespace.task_path().expect("Task path should exist");

        // Create same-named artifact in both locations
        let thread_wrapper = ArtifactWrapper::new(filesystem.clone(), thread_path.clone())
            .await
            .expect("Failed to create thread wrapper");

        thread_wrapper
            .save_artifact(duplicate_filename, r#"{"source": "thread"}"#)
            .await
            .expect("Failed to save thread artifact");

        let task_wrapper = ArtifactWrapper::new(filesystem.clone(), task_path.clone())
            .await
            .expect("Failed to create task wrapper");

        task_wrapper
            .save_artifact(duplicate_filename, r#"{"source": "task"}"#)
            .await
            .expect("Failed to save task artifact");

        // Use all_paths() to check both locations
        let mut all_artifacts = Vec::new();
        let mut seen_filenames = std::collections::HashSet::new();

        let paths_to_check = task_namespace.all_paths();
        for path_id in paths_to_check {
            let wrapper = ArtifactWrapper::new(filesystem.clone(), path_id)
                .await
                .expect("Failed to create wrapper");

            if let Ok(entries) = wrapper.list_artifacts().await {
                for e in entries {
                    if !seen_filenames.contains(&e.name) {
                        seen_filenames.insert(e.name.clone());
                        all_artifacts.push(e.name.clone());
                    }
                }
            }
        }

        // Should only have one instance of duplicate.json
        let duplicate_count = all_artifacts
            .iter()
            .filter(|&name| name == duplicate_filename)
            .count();
        assert_eq!(
            duplicate_count, 1,
            "Should only return duplicate filename once"
        );
    }
}
