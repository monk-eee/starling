fn main() {
    let asset = "web/dist/index.html";
    println!("cargo:rerun-if-changed={asset}");
    assert!(
        std::path::Path::new(asset).is_file(),
        "Build the embedded dashboard first: npm --prefix web ci, then npm --prefix web run build"
    );
}
