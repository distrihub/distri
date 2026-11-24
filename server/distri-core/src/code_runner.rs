//! [`LocalDenoRunner`]: runs workflow `code` steps with the `deno` binary on this machine.
//!
//! For the OSS server and the CLI, where the person running distri owns the machine. The
//! module runs under the `@distri/tool` harness with `--no-prompt`, a cleared environment,
//! no read/write/env/run/ffi permission and `--allow-net` only for the hosts its frontmatter
//! declares. Multi-tenant hosts (distri-cloud) run code in a sandbox instead and only use
//! this runner when an operator turns it on. The sandbox runner shares the harness
//! ([`HARNESS`]), the import map ([`import_map_json`]), the argv ([`deno_code_argv`]) and the
//! output contract ([`parse_code_output`]) from here, so both run a module the same way.

use async_trait::async_trait;
use distri_types::{CodeRunOutput, CodeRunRequest, CodeRunner};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;

/// The `@distri/tool` harness files (name, content), written next to each other.
pub const HARNESS: &[(&str, &str)] = &[
    (
        "mod.ts",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../distri-tool/mod.ts"
        )),
    ),
    (
        "run.ts",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../distri-tool/run.ts"
        )),
    ),
    (
        "runtime.ts",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../distri-tool/runtime.ts"
        )),
    ),
];

/// The first stdin line that carries a step's run token to the harness (`run.ts`), before the
/// input document.
pub const SDK_TOKEN_LINE: &str = "__distri_sdk_token__ ";

/// The harness's third argument for a step whose SDK calls are relayed by the host (stdio).
pub const SDK_RELAY: &str = "relay";
/// The stderr line prefix of a relayed SDK call (`__distri_sdk_call__ {"id", "call"}`).
pub const SDK_CALL_LINE: &str = "__distri_sdk_call__ ";

/// What a run writes to the harness's stdin when the script reaches the SDK over the network
/// (the cloud sandbox, `http` mode): the run token line, then the input document.
pub fn code_stdin(request: &CodeRunRequest) -> Vec<u8> {
    let mut out = Vec::new();
    if let Some(sdk) = &request.sdk {
        out.extend_from_slice(SDK_TOKEN_LINE.as_bytes());
        out.extend_from_slice(sdk.token.as_bytes());
        out.push(b'\n');
    }
    out.extend(serde_json::to_vec(&request.input).unwrap_or_else(|_| b"null".to_vec()));
    out
}

/// The hosts a run may reach when the script reaches the SDK over the network (the cloud
/// sandbox): the module's declared hosts plus the SDK's `host:port`. A runner that relays SDK
/// calls ([`LocalDenoRunner`]) grants the declared hosts only.
pub fn code_net(request: &CodeRunRequest) -> Vec<String> {
    let mut net: Vec<String> = request.permissions.net.iter().cloned().collect();
    if let Some(host) = request.sdk.as_ref().and_then(|s| s.net_host()) {
        if !net.iter().any(|h| h == "*" || *h == host) {
            net.push(host);
        }
    }
    net
}

/// The most stderr kept per run, in lines.
pub const MAX_LOG_LINES: usize = 200;

/// The import map that points `@distri/tool` (and `jsr:@distri/tool`) at the harness's
/// `mod.ts` (`mod_url`, a `file://` URL), and `@distri/runtime` (the script SDK) at the
/// `runtime.ts` next to it.
pub fn import_map_json(mod_url: &str) -> serde_json::Value {
    let runtime_url = match mod_url.rsplit_once('/') {
        Some((dir, _)) => format!("{dir}/runtime.ts"),
        None => "runtime.ts".to_string(),
    };
    serde_json::json!({"imports": {
        "jsr:@distri/tool": mod_url,
        "@distri/tool": mod_url,
        "jsr:@distri/runtime": runtime_url,
        "@distri/runtime": runtime_url,
    }})
}

/// The output contract of one run: stdout is the JSON document (`{"output": text}` when it
/// is not JSON), stderr lines are logs (capped at [`MAX_LOG_LINES`]), a non-zero exit is an
/// error carrying stderr's last lines (or stdout when stderr is empty).
pub fn parse_code_output(
    name: &str,
    success: bool,
    exit_code: i64,
    stdout: &str,
    stderr: &str,
    duration_ms: u64,
) -> Result<CodeRunOutput, String> {
    let stdout = stdout.trim().to_string();
    let logs: Vec<String> = stderr
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(MAX_LOG_LINES)
        .map(String::from)
        .collect();
    if !success {
        let tail = logs
            .iter()
            .rev()
            .take(12)
            .rev()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        return Err(format!(
            "code '{name}' exited with {exit_code}: {}",
            if tail.is_empty() { stdout } else { tail }
        ));
    }
    let value = serde_json::from_str::<serde_json::Value>(&stdout)
        .unwrap_or_else(|_| serde_json::json!({ "output": stdout }));
    Ok(CodeRunOutput {
        value,
        logs,
        duration_ms,
    })
}

pub struct LocalDenoRunner {
    binary: PathBuf,
    /// Where the harness, import map and `DENO_DIR` live.
    root: PathBuf,
}

impl LocalDenoRunner {
    /// `DISTRI_DENO` or `deno` on `PATH`; `None` when there is none.
    pub fn detect() -> Option<Self> {
        let binary = std::env::var_os("DISTRI_DENO")
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .filter(|p| p.is_file())
            .or_else(|| find_on_path("deno"))?;
        Some(Self::new(
            binary,
            std::env::temp_dir().join("distri-code-runner"),
        ))
    }

    pub fn new(binary: PathBuf, root: PathBuf) -> Self {
        Self { binary, root }
    }

    fn prepare(&self) -> std::io::Result<(PathBuf, PathBuf)> {
        let tool_dir = self.root.join("tool");
        std::fs::create_dir_all(&tool_dir)?;
        for (name, content) in HARNESS {
            let p = tool_dir.join(name);
            if std::fs::read_to_string(&p).ok().as_deref() != Some(*content) {
                write_atomic(&p, content.as_bytes())?;
            }
        }
        let mod_url = format!("file://{}", tool_dir.join("mod.ts").display());
        let map = import_map_json(&mod_url);
        let import_map = self.root.join("import_map.json");
        let body = serde_json::to_vec_pretty(&map).unwrap_or_default();
        if std::fs::read(&import_map).ok().as_deref() != Some(body.as_slice()) {
            write_atomic(&import_map, &body)?;
        }
        Ok((tool_dir.join("run.ts"), import_map))
    }
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|d| d.join(name))
            .find(|p| p.is_file())
    })
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// The `deno run` argv for one run: a pure function so tests can pin it.
pub fn deno_code_argv(
    import_map: &Path,
    harness: &Path,
    script: &Path,
    cwd: &Path,
    net: &[String],
) -> Vec<String> {
    deno_code_argv_sdk(import_map, harness, script, cwd, net, None)
}

/// [`deno_code_argv`], with the script SDK's URL as the harness's third argument (the run token
/// goes on stdin, see [`code_stdin`]). `net` must already hold the SDK's host ([`code_net`]).
pub fn deno_code_argv_sdk(
    import_map: &Path,
    harness: &Path,
    script: &Path,
    cwd: &Path,
    net: &[String],
    sdk_url: Option<&str>,
) -> Vec<String> {
    let mut argv = vec![
        "run".to_string(),
        "--no-prompt".to_string(),
        "--quiet".to_string(),
        "--no-lock".to_string(),
        format!("--import-map={}", import_map.display()),
        // The harness imports the module dynamically: read access to exactly the module
        // file and the harness folder, nothing else.
        format!(
            "--allow-read={},{}",
            script.display(),
            harness.parent().unwrap_or(harness).display()
        ),
    ];
    if net.iter().any(|h| h == "*") {
        argv.push("--allow-net".to_string());
    } else if !net.is_empty() {
        argv.push(format!("--allow-net={}", net.join(",")));
    }
    argv.push(harness.display().to_string());
    argv.push(format!("file://{}", script.display()));
    argv.push(cwd.display().to_string());
    if let Some(url) = sdk_url {
        argv.push(url.to_string());
    }
    argv
}

#[async_trait]
impl CodeRunner for LocalDenoRunner {
    async fn run_code(&self, request: CodeRunRequest) -> Result<CodeRunOutput, String> {
        let (harness, import_map) = self
            .prepare()
            .map_err(|e| format!("code runner: prepare harness: {e}"))?;
        let run_dir = self
            .root
            .join("runs")
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&run_dir).map_err(|e| format!("code runner: {e}"))?;
        let script = run_dir.join("module.ts");
        std::fs::write(&script, &request.source).map_err(|e| format!("code runner: {e}"))?;

        // Only the module's declared hosts: SDK calls are relayed through stdio by this process,
        // so the script never gets network access to the host (on the OSS server, a local API
        // that may have no auth) and never sees the run token.
        let net: Vec<String> = request.permissions.net.iter().cloned().collect();
        let mut cmd = tokio::process::Command::new(&self.binary);
        cmd.env_clear()
            .env("DENO_DIR", self.root.join("deno_dir"))
            .env("NO_COLOR", "1")
            .env("DENO_NO_UPDATE_CHECK", "1")
            .current_dir(&run_dir)
            .args(deno_code_argv_sdk(
                &import_map,
                &harness,
                &script,
                &run_dir,
                &net,
                request.sdk.as_ref().map(|_| SDK_RELAY),
            ))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        // HOME is needed by deno to resolve its cache on some platforms.
        if let Some(home) = std::env::var_os("HOME") {
            cmd.env("HOME", home);
        }

        let started = Instant::now();
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("code runner: start deno: {e}"))?;
        let timeout = Duration::from_secs(request.timeout_secs.max(1));
        let outcome = tokio::time::timeout(timeout, drive(&mut child, &request)).await;
        if outcome.is_err() {
            let _ = child.kill().await;
        }
        let _ = std::fs::remove_dir_all(&run_dir);
        let (status, stdout, stderr) = match outcome {
            Ok(Ok(o)) => o,
            Ok(Err(e)) => return Err(format!("code '{}' failed to run: {e}", request.name)),
            Err(_) => {
                return Err(format!(
                    "code '{}' timed out after {}s",
                    request.name,
                    timeout.as_secs()
                ))
            }
        };
        parse_code_output(
            &request.name,
            status.success(),
            status.code().unwrap_or(-1) as i64,
            &stdout,
            &stderr,
            started.elapsed().as_millis() as u64,
        )
    }
}

/// Feed the input, relay SDK calls (when the step has the SDK) and collect stdout and stderr
/// (without the relay's call lines) until the process exits.
async fn drive(
    child: &mut tokio::process::Child,
    request: &CodeRunRequest,
) -> std::io::Result<(std::process::ExitStatus, String, String)> {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
    let mut stdin = child.stdin.take();
    let mut stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");

    let input = serde_json::to_vec(&request.input).unwrap_or_else(|_| b"null".to_vec());
    if let Some(w) = stdin.as_mut() {
        let _ = w.write_all(&input).await;
        if request.sdk.is_some() {
            // Relay mode: the input is the first line; answers follow on stdin.
            let _ = w.write_all(b"\n").await;
            let _ = w.flush().await;
        }
    }
    if request.sdk.is_none() {
        drop(stdin.take());
    }

    let out_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf).await;
        String::from_utf8_lossy(&buf).to_string()
    });

    let sdk = request.sdk.clone();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(request.timeout_secs.max(1) + 30))
        .no_proxy()
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    let mut logs = String::new();
    let mut lines = BufReader::new(stderr).lines();
    while let Some(line) = lines.next_line().await? {
        let Some(rest) = line.strip_prefix(SDK_CALL_LINE) else {
            logs.push_str(&line);
            logs.push('\n');
            continue;
        };
        let (Some(sdk), Some(w)) = (sdk.as_ref(), stdin.as_mut()) else {
            continue;
        };
        let answer = relay_call(&client, sdk, rest).await;
        let mut line = serde_json::to_vec(&answer).unwrap_or_default();
        line.push(b'\n');
        let _ = w.write_all(&line).await;
        let _ = w.flush().await;
    }
    drop(stdin);
    let status = child.wait().await?;
    let stdout = out_task.await.unwrap_or_default();
    Ok((status, stdout, logs))
}

/// One relayed SDK call: `{"id", "call"}` from the script, answered as `{"id", "ok", …}` from
/// the host's SDK route (the run token is added here; the script never has it).
async fn relay_call(
    client: &reqwest::Client,
    sdk: &distri_types::script_sdk::CodeSdkAccess,
    raw: &str,
) -> serde_json::Value {
    let msg: serde_json::Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(e) => {
            return serde_json::json!({"id": null, "ok": false, "code": "invalid", "error": format!("bad call line: {e}")})
        }
    };
    let id = msg.get("id").cloned().unwrap_or(serde_json::Value::Null);
    let call = msg.get("call").cloned().unwrap_or(serde_json::Value::Null);
    let res = client
        .post(format!("{}/call", sdk.url.trim_end_matches('/')))
        .bearer_auth(&sdk.token)
        .json(&call)
        .send()
        .await;
    let mut answer = match res {
        Ok(r) => r.json::<serde_json::Value>().await.unwrap_or_else(|e| {
            serde_json::json!({"ok": false, "code": "failed", "error": format!("the SDK answered no JSON: {e}")})
        }),
        Err(e) => serde_json::json!({"ok": false, "code": "failed",
            "error": distri_types::script_sdk::clean_error(&format!("the SDK is unreachable: {e}"))}),
    };
    if let Some(obj) = answer.as_object_mut() {
        obj.insert("id".into(), id);
    }
    answer
}

#[cfg(test)]
mod tests {
    use super::*;
    use distri_types::CodePermissions;

    #[test]
    fn argv_grants_only_declared_hosts() {
        let argv = deno_code_argv(
            Path::new("/r/import_map.json"),
            Path::new("/r/tool/run.ts"),
            Path::new("/r/runs/x/module.ts"),
            Path::new("/r/runs/x"),
            &["api.acme.com".to_string()],
        );
        assert!(argv.contains(&"--allow-net=api.acme.com".to_string()));
        assert!(argv.contains(&"--no-prompt".to_string()));
        assert!(argv.contains(&"--allow-read=/r/runs/x/module.ts,/r/tool".to_string()));
        assert!(!argv.iter().any(|a| a.starts_with("--allow-env")
            || a.starts_with("--allow-run")
            || a.starts_with("--allow-write")));
        let none = deno_code_argv(
            Path::new("/m"),
            Path::new("/h"),
            Path::new("/s"),
            Path::new("/c"),
            &[],
        );
        assert!(!none.iter().any(|a| a.starts_with("--allow-net")));
    }

    #[test]
    fn output_contract() {
        let ok = parse_code_output("m", true, 0, "{\"a\":1}\n", "log one\n\nlog two\n", 5).unwrap();
        assert_eq!(ok.value, serde_json::json!({"a": 1}));
        assert_eq!(ok.logs, vec!["log one", "log two"]);
        let text = parse_code_output("m", true, 0, "hello", "", 0).unwrap();
        assert_eq!(text.value, serde_json::json!({"output": "hello"}));
        let err = parse_code_output("m", false, 1, "", "Error: boom\n", 0).unwrap_err();
        assert_eq!(err, "code 'm' exited with 1: Error: boom");
    }

    fn request(source: &str) -> CodeRunRequest {
        CodeRunRequest {
            source: source.to_string(),
            name: "test".into(),
            input: serde_json::json!({"n": 2}),
            permissions: CodePermissions::default(),
            timeout_secs: 60,
            workspace_id: None,
            owner_id: "wf".into(),
            sdk: None,
        }
    }

    #[tokio::test]
    async fn runs_default_export_with_input() {
        let Some(runner) = LocalDenoRunner::detect() else {
            eprintln!("deno not installed; skipping");
            return;
        };
        let out = runner
            .run_code(request(
                "export default async function (input: { n: number }) { return { doubled: input.n * 2 }; }",
            ))
            .await
            .unwrap();
        assert_eq!(out.value, serde_json::json!({"doubled": 4}));
    }

    #[tokio::test]
    async fn network_is_denied_without_permission() {
        let Some(runner) = LocalDenoRunner::detect() else {
            return;
        };
        let err = runner
            .run_code(request(
                "export default async function () { await fetch('https://example.com'); return {}; }",
            ))
            .await
            .unwrap_err();
        assert!(err.contains("NotCapable") || err.contains("net"), "{err}");
    }

    #[tokio::test]
    async fn a_throw_is_an_error() {
        let Some(runner) = LocalDenoRunner::detect() else {
            return;
        };
        let err = runner
            .run_code(request(
                "export default function () { throw new Error('boom'); }",
            ))
            .await
            .unwrap_err();
        assert!(err.contains("boom"), "{err}");
    }

    /// A one-route HTTP server for the SDK: answers every `POST …/call` with
    /// `{ok: true, result: {echo: <op>}}` and keeps `(authorization, body)` of each request.
    async fn sdk_server() -> (String, std::sync::Arc<std::sync::Mutex<Vec<(String, serde_json::Value)>>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let seen = seen2.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    let (head, body) = loop {
                        let n = sock.read(&mut chunk).await.unwrap_or(0);
                        if n == 0 { return; }
                        buf.extend_from_slice(&chunk[..n]);
                        let text = String::from_utf8_lossy(&buf).to_string();
                        if let Some(i) = text.find("\r\n\r\n") {
                            let head = text[..i].to_string();
                            let len = head.lines()
                                .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                                .unwrap_or(0);
                            while buf.len() < i + 4 + len {
                                let n = sock.read(&mut chunk).await.unwrap_or(0);
                                if n == 0 { break; }
                                buf.extend_from_slice(&chunk[..n]);
                            }
                            break (head, buf[i + 4..].to_vec());
                        }
                    };
                    let auth = head.lines()
                        .find_map(|l| l.to_ascii_lowercase().starts_with("authorization:").then(|| l[14..].trim().to_string()))
                        .unwrap_or_default();
                    let body: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
                    let op = body["op"].as_str().unwrap_or("").to_string();
                    seen.lock().unwrap().push((auth, body));
                    let reply = serde_json::json!({"ok": true, "result": {"echo": op}}).to_string();
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        reply.len(), reply
                    );
                    let _ = sock.write_all(resp.as_bytes()).await;
                });
            }
        });
        (format!("http://{addr}/v1/workflows/sdk"), seen)
    }

    #[test]
    fn stdin_carries_the_token_and_net_the_sdk_host() {
        let mut r = request("x");
        assert_eq!(code_stdin(&r), br#"{"n":2}"#.to_vec());
        assert!(code_net(&r).is_empty());
        r.sdk = Some(distri_types::script_sdk::CodeSdkAccess {
            url: "http://127.0.0.1:9/v1/workflows/sdk".into(),
            token: "dsr_t".into(),
        });
        assert_eq!(
            String::from_utf8(code_stdin(&r)).unwrap(),
            "__distri_sdk_token__ dsr_t\n{\"n\":2}"
        );
        assert_eq!(code_net(&r), vec!["127.0.0.1:9".to_string()]);
        let mut https = r.clone();
        https.sdk = Some(distri_types::script_sdk::CodeSdkAccess {
            url: "https://api.distri.dev/v1/workflows/sdk".into(),
            token: "t".into(),
        });
        assert_eq!(code_net(&https), vec!["api.distri.dev:443".to_string()]);
        let argv = deno_code_argv_sdk(
            Path::new("/m"), Path::new("/h"), Path::new("/s"), Path::new("/c"),
            &code_net(&r), Some("http://127.0.0.1:9/v1/workflows/sdk"),
        );
        assert_eq!(argv.last().unwrap(), "http://127.0.0.1:9/v1/workflows/sdk");
        assert!(!argv.iter().any(|a| a.contains("dsr_t")), "the token is never on the command line");
        let map = import_map_json("file:///r/tool/mod.ts");
        assert_eq!(map["imports"]["@distri/runtime"], "file:///r/tool/runtime.ts");
    }

    #[tokio::test]
    async fn real_deno_script_reaches_the_sdk_with_its_run_token() {
        let Some(runner) = LocalDenoRunner::detect() else {
            eprintln!("deno not installed; skipping");
            return;
        };
        let (url, seen) = sdk_server().await;
        let mut r = request(
            r#"import { distri } from "@distri/runtime";
export default async function (input: { n: number }) {
  const s = await distri.state.set({ n: input.n });
  const m = await distri.notify("hello", [[{ kind: "url", label: "Open", url: "https://distri.dev" }]]);
  return { s, m };
}"#,
        );
        r.sdk = Some(distri_types::script_sdk::CodeSdkAccess { url, token: "dsr_test_token".into() });
        let out = runner.run_code(r).await.unwrap();
        assert_eq!(out.value["s"]["echo"], "state_set");
        assert_eq!(out.value["m"]["echo"], "notify");
        let seen = seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].0, "Bearer dsr_test_token");
        assert_eq!(seen[0].1, serde_json::json!({"op": "state_set", "patch": {"n": 2}}));
        assert_eq!(seen[1].1["text"], "hello");

        // The script itself cannot reach the SDK's host (calls are relayed by this process),
        // and the token is nowhere in its reach.
        let (url2, seen2) = sdk_server().await;
        let mut probe = request(&format!(
            r#"import {{ distri }} from "@distri/runtime";
export default async function () {{
  let direct = "reached";
  try {{ await fetch("{url2}/call", {{ method: "POST", body: "{{}}" }}); }} catch (e) {{ direct = String(e); }}
  const args = JSON.stringify(Deno.args);
  const keys = JSON.stringify(Object.getOwnPropertyNames(globalThis).filter((k) => k.includes("distri")));
  const s = await distri.state.get();
  return {{ direct, args, keys, s }};
}}"#
        ));
        probe.sdk = Some(distri_types::script_sdk::CodeSdkAccess {
            url: url2,
            token: "dsr_secret_probe".into(),
        });
        let out = runner.run_code(probe).await.unwrap();
        let direct = out.value["direct"].as_str().unwrap_or_default();
        assert!(direct.contains("NotCapable") || direct.contains("net"), "{direct}");
        assert!(!out.value.to_string().contains("dsr_secret_probe"), "{}", out.value);
        assert_eq!(out.value["s"]["echo"], "state_get");
        let seen2 = seen2.lock().unwrap().clone();
        assert_eq!(seen2.len(), 1, "only the relayed call reached the host");
        assert_eq!(seen2[0].0, "Bearer dsr_secret_probe");

        // Without the SDK, a call throws a clear error.
        let err = runner
            .run_code(request(
                r#"import { distri } from "@distri/runtime";
export default async function () { await distri.state.get(); return {}; }"#,
            ))
            .await
            .unwrap_err();
        assert!(err.contains("offers no script SDK"), "{err}");
    }
}
