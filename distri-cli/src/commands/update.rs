use anyhow::Result;

use crate::launcher::resolve::{resolve_server, resolve_ui, ResolveOpts};

pub async fn run(allow_pre: bool) -> Result<()> {
    let http = reqwest::Client::new();
    let opts = ResolveOpts {
        pinned_version: None,
        allow_pre,
        force_refresh: true,
    };
    println!("Resolving distri-server (force refresh)...");
    let server = resolve_server(&http, &opts).await?;
    println!("  installed: {}", server.display());

    println!("Resolving distri-ui (force refresh)...");
    let ui = resolve_ui(&http, &opts).await?;
    println!("  installed: {}", ui.display());

    Ok(())
}
