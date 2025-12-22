fn main() {
    // This build script enables wit-bindgen to generate bindings from our WIT files
    println!("cargo:rerun-if-changed=wit");
}
