use anyhow::{Context, Result};
use std::fs;
use std::process::Command;
use uuid::Uuid;

pub struct PythonOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
}

pub fn execute_python(code: &str) -> Result<PythonOutput> {
    // Create a temporary directory for the Python file
    let tmp_dir = std::env::temp_dir().join(format!("python-exec-{}", Uuid::new_v4()));
    fs::create_dir_all(&tmp_dir)?;

    let python_file = tmp_dir.join("script.py");
    fs::write(&python_file, code)?;

    // Run the Docker container with a specified platform to suppress warnings
    let output = Command::new("docker")
        .args([
            "run",
            "--rm",
            "--platform",
            "linux/amd64",
            "--network",
            "none",
            "--memory",
            "512m",
            "--memory-swap",
            "512m",
            "--cpus",
            "1",
            "-v",
            &format!("{}:/code:ro", tmp_dir.display()),
            "python:3.9-slim",
            "python",
            "/code/script.py",
        ])
        .output()
        .context("Failed to execute Docker command")?;

    // Clean up
    fs::remove_dir_all(tmp_dir)?;

    Ok(PythonOutput {
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        exit_code: output.status.code(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_python_execution() {
        let code = r#"
print("Hello, World!")
for i in range(3):
    print(f"Number: {i}")
"#;

        let result = execute_python(code).unwrap();
        println!("{}", result.stdout.to_string());
        println!("{}", result.stderr.to_string());
        assert_eq!(result.exit_code, Some(0));
        assert_eq!(
            result.stdout.trim(),
            "Hello, World!\nNumber: 0\nNumber: 1\nNumber: 2"
        );
        println!("{}", result.stderr.to_string());
        assert!(result.stderr.is_empty());
    }

    #[test]
    fn test_python_syntax_error() {
        let code = "print('Unclosed string";

        let result = execute_python(code).unwrap();

        assert_ne!(result.exit_code, Some(0));
        assert!(!result.stderr.is_empty());
    }
}
