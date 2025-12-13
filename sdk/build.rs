fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Proto file is located in ../proto/
    let proto_path = "../proto";
    let proto_file = format!("{}/flovyn.proto", proto_path);

    // Check if proto file exists
    if std::path::Path::new(&proto_file).exists() {
        tonic_build::configure()
            .build_server(false)
            .build_client(true)
            .out_dir("src/generated")
            .compile(&[&proto_file], &[proto_path])?;

        println!("cargo:rerun-if-changed={}", proto_file);
    } else {
        // Proto file not found - skip generation for now
        // This allows the SDK to compile without the proto file
        println!(
            "cargo:warning=Proto file not found at {}, skipping gRPC code generation",
            proto_file
        );
    }

    Ok(())
}
