/// Integration tests for distri-cli coding capabilities.
///
/// These tests verify that distri-cli can perform coding tasks by connecting
/// to a running distri-cloud instance. They test the full round-trip:
/// CLI → orchestrator → agent → tool calls → local execution → result.
///
/// Enable with: DISTRI_CODING_TEST=1 DISTRI_BASE_URL=http://localhost:8081/v1
///
/// The tests use the `coder` agent which delegates coding tasks to a shell.
/// distri-cli registers local filesystem tools + execute_command as external
/// tools, so the agent can read/write files and run commands on the machine
/// where the CLI runs.
use anyhow::{Context, Result};
use std::env;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

static CODING_LOCK: Mutex<()> = Mutex::new(());

fn coding_base_url() -> Option<String> {
    if env::var("DISTRI_CODING_TEST").unwrap_or_default() != "1" {
        return None;
    }
    Some(
        env::var("DISTRI_BASE_URL")
            .unwrap_or_else(|_| "http://localhost:8081/v1".to_string()),
    )
}

fn distri_binary() -> PathBuf {
    // Use the binary built by cargo in the same target directory
    let mut path = PathBuf::from(env!("CARGO_BIN_EXE_distri"));
    if !path.exists() {
        // Fallback: look next to the test binary
        path = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("distri");
    }
    path
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .to_path_buf()
}

/// Run distri CLI with the coder agent and a task, capturing output.
/// Uses a temp workspace so file operations are isolated.
fn run_coder_task(base_url: &str, task: &str, workspace: &std::path::Path) -> Result<String> {
    let output = Command::new(distri_binary())
        .arg("--base-url")
        .arg(base_url)
        .arg("run")
        .arg("--agent")
        .arg("coder")
        .arg("--task")
        .arg(task)
        .env("DISTRI_WORKSPACE", workspace.to_string_lossy().as_ref())
        .output()
        .with_context(|| format!("running coder with task: {}", task))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if !output.status.success() {
        anyhow::bail!(
            "coder task failed (status={})\ntask: {}\nstdout:\n{}\nstderr:\n{}",
            output.status,
            task,
            &stdout[..stdout.len().min(2000)],
            &stderr[..stderr.len().min(2000)]
        );
    }

    Ok(format!("{}{}", stdout, stderr))
}

// ============================================================
// Coding task tests
// ============================================================

/// Test: Agent can write a simple Python file.
#[test]
fn coding_write_hello_world_python() -> Result<()> {
    let _lock = CODING_LOCK.lock().unwrap();
    let Some(base_url) = coding_base_url() else {
        eprintln!("Skipping coding test; set DISTRI_CODING_TEST=1 to enable");
        return Ok(());
    };

    let workspace = tempfile::tempdir().context("creating temp workspace")?;
    let output = run_coder_task(
        &base_url,
        "Write a Python file called hello.py that prints 'Hello, World!' to stdout. Just write the file, nothing else.",
        workspace.path(),
    )?;

    // Verify the file was created
    let hello_py = workspace.path().join("hello.py");
    assert!(
        hello_py.exists(),
        "hello.py should have been created in workspace.\nOutput: {}",
        &output[..output.len().min(1000)]
    );

    // Verify content
    let content = std::fs::read_to_string(&hello_py)?;
    assert!(
        content.contains("Hello, World!") || content.contains("Hello, world!"),
        "hello.py should contain Hello World. Got: {}",
        content
    );

    Ok(())
}

/// Test: Agent can write and execute a simple program.
#[test]
fn coding_write_and_run_program() -> Result<()> {
    let _lock = CODING_LOCK.lock().unwrap();
    let Some(base_url) = coding_base_url() else {
        eprintln!("Skipping coding test; set DISTRI_CODING_TEST=1 to enable");
        return Ok(());
    };

    let workspace = tempfile::tempdir().context("creating temp workspace")?;
    let output = run_coder_task(
        &base_url,
        "Write a bash script called add.sh that takes two numbers as arguments and prints their sum. Then run it with arguments 3 and 4 to verify it outputs 7.",
        workspace.path(),
    )?;

    let add_sh = workspace.path().join("add.sh");
    assert!(
        add_sh.exists(),
        "add.sh should have been created.\nOutput: {}",
        &output[..output.len().min(1000)]
    );

    // The output should mention 7 (the result of 3+4)
    assert!(
        output.contains("7"),
        "output should contain the result 7.\nOutput: {}",
        &output[..output.len().min(1000)]
    );

    Ok(())
}

/// Test: Agent can read an existing file and answer questions about it.
#[test]
fn coding_read_and_analyze_file() -> Result<()> {
    let _lock = CODING_LOCK.lock().unwrap();
    let Some(base_url) = coding_base_url() else {
        eprintln!("Skipping coding test; set DISTRI_CODING_TEST=1 to enable");
        return Ok(());
    };

    let workspace = tempfile::tempdir().context("creating temp workspace")?;

    // Pre-create a file for the agent to read
    let code = r#"
def fibonacci(n):
    if n <= 1:
        return n
    return fibonacci(n-1) + fibonacci(n-2)

for i in range(10):
    print(fibonacci(i))
"#;
    std::fs::write(workspace.path().join("fib.py"), code)?;

    let output = run_coder_task(
        &base_url,
        "Read fib.py and tell me what the 10th number it prints will be. Just give me the number.",
        workspace.path(),
    )?;

    // fibonacci(9) = 34
    assert!(
        output.contains("34"),
        "output should contain 34 (fibonacci(9)).\nOutput: {}",
        &output[..output.len().min(1000)]
    );

    Ok(())
}

/// Test: Agent can edit an existing file.
#[test]
fn coding_edit_existing_file() -> Result<()> {
    let _lock = CODING_LOCK.lock().unwrap();
    let Some(base_url) = coding_base_url() else {
        eprintln!("Skipping coding test; set DISTRI_CODING_TEST=1 to enable");
        return Ok(());
    };

    let workspace = tempfile::tempdir().context("creating temp workspace")?;

    // Pre-create a file with a bug
    let code = r#"def greet(name):
    return "Hello, " + namee  # typo: namee instead of name

print(greet("World"))
"#;
    std::fs::write(workspace.path().join("greet.py"), code)?;

    let output = run_coder_task(
        &base_url,
        "Fix the bug in greet.py (there's a typo 'namee' that should be 'name') and verify it works by running it.",
        workspace.path(),
    )?;

    // Verify the file was fixed
    let content = std::fs::read_to_string(workspace.path().join("greet.py"))?;
    assert!(
        !content.contains("namee"),
        "greet.py should no longer contain the typo 'namee'.\nContent: {}",
        content
    );

    // Output should show successful execution
    assert!(
        output.contains("Hello, World"),
        "output should contain 'Hello, World' from running the fixed script.\nOutput: {}",
        &output[..output.len().min(1000)]
    );

    Ok(())
}

/// Test: Agent can create a multi-file project.
#[test]
fn coding_create_multifile_project() -> Result<()> {
    let _lock = CODING_LOCK.lock().unwrap();
    let Some(base_url) = coding_base_url() else {
        eprintln!("Skipping coding test; set DISTRI_CODING_TEST=1 to enable");
        return Ok(());
    };

    let workspace = tempfile::tempdir().context("creating temp workspace")?;
    let output = run_coder_task(
        &base_url,
        "Create a simple Node.js project with two files: 1) math.js that exports an add(a,b) function, and 2) index.js that imports add from math.js and prints add(10, 20). Just create the files.",
        workspace.path(),
    )?;

    let math_js = workspace.path().join("math.js");
    let index_js = workspace.path().join("index.js");

    assert!(
        math_js.exists(),
        "math.js should exist.\nOutput: {}",
        &output[..output.len().min(1000)]
    );
    assert!(
        index_js.exists(),
        "index.js should exist.\nOutput: {}",
        &output[..output.len().min(1000)]
    );

    let math_content = std::fs::read_to_string(&math_js)?;
    assert!(
        math_content.contains("add") && math_content.contains("export"),
        "math.js should export an add function.\nContent: {}",
        math_content
    );

    Ok(())
}

/// Test: Agent can search within files.
#[test]
fn coding_search_in_files() -> Result<()> {
    let _lock = CODING_LOCK.lock().unwrap();
    let Some(base_url) = coding_base_url() else {
        eprintln!("Skipping coding test; set DISTRI_CODING_TEST=1 to enable");
        return Ok(());
    };

    let workspace = tempfile::tempdir().context("creating temp workspace")?;

    // Create several files with different content
    std::fs::write(
        workspace.path().join("config.py"),
        "DATABASE_URL = 'postgres://localhost/mydb'\nDEBUG = True\n",
    )?;
    std::fs::write(
        workspace.path().join("app.py"),
        "from config import DATABASE_URL\nprint(DATABASE_URL)\n",
    )?;
    std::fs::write(
        workspace.path().join("README.md"),
        "# My App\nA simple Python app.\n",
    )?;

    let output = run_coder_task(
        &base_url,
        "Search the workspace to find which file contains DATABASE_URL. Just tell me the filename.",
        workspace.path(),
    )?;

    assert!(
        output.contains("config.py"),
        "should identify config.py as containing DATABASE_URL.\nOutput: {}",
        &output[..output.len().min(1000)]
    );

    Ok(())
}

/// Test: Agent can list directory contents.
#[test]
fn coding_list_directory() -> Result<()> {
    let _lock = CODING_LOCK.lock().unwrap();
    let Some(base_url) = coding_base_url() else {
        eprintln!("Skipping coding test; set DISTRI_CODING_TEST=1 to enable");
        return Ok(());
    };

    let workspace = tempfile::tempdir().context("creating temp workspace")?;
    std::fs::write(workspace.path().join("file1.txt"), "a")?;
    std::fs::write(workspace.path().join("file2.txt"), "b")?;
    std::fs::create_dir_all(workspace.path().join("src"))?;
    std::fs::write(workspace.path().join("src/main.rs"), "fn main() {}")?;

    let output = run_coder_task(
        &base_url,
        "List all files in the workspace including subdirectories. Just list the filenames.",
        workspace.path(),
    )?;

    assert!(
        output.contains("file1.txt") && output.contains("file2.txt"),
        "should list file1.txt and file2.txt.\nOutput: {}",
        &output[..output.len().min(1000)]
    );

    Ok(())
}
