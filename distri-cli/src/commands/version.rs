use anyhow::Result;

use crate::manifest;

pub fn run() -> Result<()> {
    let mf = manifest::read()?;
    println!("distri-cli    {}", env!("CARGO_PKG_VERSION"));
    println!(
        "distri-server {}",
        mf.server
            .as_ref()
            .map(|r| r.version.as_str())
            .unwrap_or("<not installed>"),
    );
    println!(
        "distri-ui     {}",
        mf.ui
            .as_ref()
            .map(|r| r.version.as_str())
            .unwrap_or("<not installed>"),
    );
    Ok(())
}
