fn main() -> Result<(), Box<dyn std::error::Error>> {
    let asset = "web/dist/index.html";
    println!("cargo:rerun-if-changed={asset}");
    assert!(
        std::path::Path::new(asset).is_file(),
        "Build the embedded dashboard first: npm --prefix web ci, then npm --prefix web run build"
    );
    println!("cargo:rerun-if-changed=proto/flock.proto");
    let descriptors = protox::compile(["proto/flock.proto"], ["proto"])?;
    tonic_prost_build::configure().compile_fds(descriptors)?;
    Ok(())
}
