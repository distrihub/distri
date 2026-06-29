use anyhow::Result;

use crate::manifest;

pub fn run() -> Result<()> {
    let home = manifest::distri_home()?;
    for sub in ["bin", "ui", "cache"] {
        let p = home.join(sub);
        if p.exists() {
            std::fs::remove_dir_all(&p)?;
            println!("removed {}", p.display());
        }
    }
    Ok(())
}
