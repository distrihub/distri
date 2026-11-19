//! `distri apply`: set up an agent, its workflows and where it lives from a blueprint.
//!
//! `distri apply ./samples/sre-agent --set service_url=https://api.acme.com/health`
//! `distri apply sre-agent` (a sample the server ships) · `distri apply --list`
//!
//! Applies through `POST /v1/blueprints/apply`, then prints what was created and what a
//! person still has to do (connect Telegram, connect GitHub, paste a snippet), each with its
//! link. `--wait` polls until those are done; `--test` then runs the blueprint's test.

use std::collections::BTreeMap;
use std::io::{IsTerminal, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use distri::Distri;
use distri_types::blueprint::{ApplyBlueprintRequest, ApplyBlueprintResponse, Blueprint};

pub struct ApplyArgs {
    pub target: Option<String>,
    pub set: Vec<String>,
    pub dry_run: bool,
    pub json: bool,
    pub wait: bool,
    pub test: bool,
    pub list: bool,
    pub export: bool,
}

/// `k=v` pairs into a map.
pub fn parse_sets(sets: &[String]) -> Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    for s in sets {
        let Some((k, v)) = s.split_once('=') else {
            bail!("--set takes name=value, got '{s}'");
        };
        out.insert(k.trim().to_string(), v.trim().to_string());
    }
    Ok(out)
}

async fn load_target(target: &str) -> Result<(Option<Blueprint>, Option<String>)> {
    let path = Path::new(target);
    if path.is_dir() {
        let bp = Blueprint::load_dir(path).with_context(|| format!("load {target}"))?;
        return Ok((Some(bp), None));
    }
    if target.starts_with("http://") || target.starts_with("https://") {
        // A published sample: `<url>/blueprint.json` (or the URL itself when it is the JSON).
        let url = if target.ends_with(".json") {
            target.to_string()
        } else {
            format!(
                "{}/blueprint.json",
                target
                    .trim_end_matches('/')
                    .trim_end_matches("/SKILL.md")
                    .trim_end_matches("/README.md")
            )
        };
        let bp: Blueprint = reqwest::get(&url)
            .await
            .with_context(|| format!("fetch {url}"))?
            .error_for_status()
            .with_context(|| format!("fetch {url}"))?
            .json()
            .await
            .with_context(|| format!("{url} is not a blueprint"))?;
        bp.validate()?;
        return Ok((Some(bp), None));
    }
    if path.exists() {
        bail!("{target} is not a folder with a blueprint.toml");
    }
    Ok((None, Some(target.to_string())))
}

pub async fn handle_apply(client: &Distri, args: ApplyArgs) -> Result<()> {
    if args.export {
        let target = args
            .target
            .clone()
            .context("--export needs a blueprint folder")?;
        let (Some(bp), _) = load_target(&target).await? else {
            bail!("--export needs a blueprint folder or URL, not a sample name");
        };
        println!("{}", serde_json::to_string_pretty(&bp)?);
        return Ok(());
    }
    if args.list || args.target.is_none() {
        let samples = client.list_blueprint_samples().await?;
        if args.json {
            println!("{}", serde_json::to_string_pretty(&samples)?);
            return Ok(());
        }
        for s in samples {
            println!("{:<18} {}", s.name, s.title.clone().unwrap_or_default());
            println!("{:<18} {}", "", s.about);
            if let Some(p) = &s.prompt {
                println!("{:<18} \"{}\"", "", p);
            }
            println!();
        }
        println!("Apply one: distri apply <name> --set <param>=<value>");
        return Ok(());
    }
    let target = args.target.clone().unwrap_or_default();
    let (blueprint, sample) = load_target(&target).await?;
    let mut params = parse_sets(&args.set)?;

    let mut resp = loop {
        let req = ApplyBlueprintRequest {
            blueprint: blueprint.clone(),
            sample: sample.clone(),
            params: params.clone(),
            dry_run: args.dry_run,
        };
        let resp = client.apply_blueprint(&req).await?;
        if resp.status == "needs_params" && !args.json && std::io::stdin().is_terminal() {
            for m in &resp.missing_params {
                let hint = m
                    .example
                    .as_ref()
                    .map(|e| format!(" (e.g. {e})"))
                    .unwrap_or_default();
                print!("{}{hint}\n> ", m.ask);
                std::io::stdout().flush().ok();
                let mut line = String::new();
                std::io::stdin().read_line(&mut line)?;
                params.insert(m.name.clone(), line.trim().to_string());
            }
            continue;
        }
        break resp;
    };

    if args.json && !args.wait && !args.test {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }
    if !args.json {
        print_response(&resp);
    }
    if resp.status == "needs_params" {
        if args.json {
            println!("{}", serde_json::to_string_pretty(&resp)?);
        }
        std::process::exit(2);
    }

    if args.wait {
        if let Some(id) = resp.install_id.clone() {
            let started = Instant::now();
            let mut last_open: Vec<String> = open_ids(&resp);
            while resp.status == "pending"
                && resp.pending.iter().any(|p| p.required && !p.done)
                && started.elapsed() < Duration::from_secs(20 * 60)
            {
                tokio::time::sleep(Duration::from_secs(3)).await;
                resp = client.get_blueprint_install(&id).await?;
                let open = open_ids(&resp);
                if open != last_open && !args.json {
                    for p in resp.pending.iter().filter(|p| p.done) {
                        if last_open.contains(&p.id) {
                            println!("✓ {}", p.title);
                        }
                    }
                }
                last_open = open;
            }
        }
    }

    if args.test {
        let Some(id) = resp.install_id.clone() else {
            bail!("nothing was applied (dry run?), so there is nothing to test");
        };
        if !args.json {
            println!("Running the test…");
        }
        let out = client.test_blueprint_install(&id).await?;
        if args.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({"apply": resp, "test": out}))?
            );
        } else {
            let ok = out.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
            let msg = out
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            println!("{} {msg}", if ok { "✓" } else { "✗" });
            if !ok {
                std::process::exit(1);
            }
        }
    } else if args.json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
    }
    Ok(())
}

fn open_ids(resp: &ApplyBlueprintResponse) -> Vec<String> {
    resp.pending
        .iter()
        .filter(|p| !p.done)
        .map(|p| p.id.clone())
        .collect()
}

fn print_response(resp: &ApplyBlueprintResponse) {
    let title = resp.title.clone().unwrap_or_else(|| resp.blueprint.clone());
    println!("{title}: {}", resp.message);
    if !resp.missing_params.is_empty() {
        println!("\nNeeds:");
        for m in &resp.missing_params {
            println!("  --set {}=…   {}", m.name, m.ask);
        }
    }
    if !resp.items.is_empty() {
        println!();
        for i in &resp.items {
            let url = i.url.as_ref().map(|u| format!("  {u}")).unwrap_or_default();
            println!("  {:<9} {:<14} {}{url}", i.action, i.kind, i.name);
        }
    }
    let open: Vec<_> = resp.pending.iter().filter(|p| !p.done).collect();
    if !open.is_empty() {
        println!("\nTo finish:");
        for p in open {
            let req = if p.required { "" } else { " (optional)" };
            println!("  • {}{req}", p.title);
            if !p.why.is_empty() {
                println!("    {}", p.why);
            }
            if let Some(u) = &p.url {
                println!("    {u}");
            }
            if let Some(s) = &p.snippet {
                println!("    {}", s.replace('\n', "\n    "));
            }
        }
    }
    if let Some(t) = &resp.test {
        if !t.expect.is_empty() {
            println!("\nThen: distri apply … --test  →  {}", t.expect);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_parse() {
        let m = parse_sets(&["a=1".into(), "url=https://x/y?z=1".into()]).unwrap();
        assert_eq!(m["a"], "1");
        assert_eq!(m["url"], "https://x/y?z=1");
        assert!(parse_sets(&["nope".into()]).is_err());
    }

    #[tokio::test]
    async fn a_folder_loads_as_a_blueprint() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../samples/sre-agent");
        let (bp, sample) = load_target(dir.to_str().unwrap()).await.unwrap();
        assert_eq!(bp.unwrap().name, "sre-agent");
        assert!(sample.is_none());
        let (bp, sample) = load_target("sre-agent").await.unwrap();
        assert!(bp.is_none());
        assert_eq!(sample.as_deref(), Some("sre-agent"));
    }
}
