#[cfg(feature = "ui")]
use static_files::resource_dir;

fn main() -> std::io::Result<()> {
    #[cfg(feature = "ui")]
    {
        let dist_path = "../../distri-ui/dist";
        println!("cargo:rerun-if-changed={}", dist_path);
        println!("cargo:rerun-if-changed=build.rs");

        if !std::path::Path::new(dist_path).exists() {
            panic!(
                "UI feature is enabled but {} is missing. Run `make build-ui` to build the frontend.",
                dist_path
            );
        }

        resource_dir(dist_path).build()?;
    }

    #[cfg(not(feature = "ui"))]
    {
        println!("cargo:rerun-if-changed=build.rs");
    }

    Ok(())
}
