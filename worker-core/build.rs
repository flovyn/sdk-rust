use std::process::Command;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Proto files are located in ../proto/ (relative to core/)
    let proto_path = "../proto";
    let flovyn_proto = format!("{}/flovyn.proto", proto_path);
    let agent_proto = format!("{}/agent.proto", proto_path);

    // Collect proto files that exist
    let mut proto_files = Vec::new();
    if std::path::Path::new(&flovyn_proto).exists() {
        proto_files.push(flovyn_proto.clone());
        println!("cargo:rerun-if-changed={}", flovyn_proto);
    }
    if std::path::Path::new(&agent_proto).exists() {
        proto_files.push(agent_proto.clone());
        println!("cargo:rerun-if-changed={}", agent_proto);
    }

    if !proto_files.is_empty() {
        tonic_build::configure()
            .build_server(false)
            .build_client(true)
            .out_dir("src/generated")
            .compile(&proto_files, &[proto_path])?;

        // Run rustfmt on generated code for consistent formatting across environments
        let generated_file = "src/generated/flovyn.v1.rs";
        if std::path::Path::new(generated_file).exists() {
            let _ = Command::new("rustfmt")
                .arg(generated_file)
                .arg("--edition")
                .arg("2021")
                .status();
        }
    } else {
        // Proto files not found - skip generation for now
        // This allows the core to compile without the proto files
        println!(
            "cargo:warning=Proto files not found at {}, skipping gRPC code generation",
            proto_path
        );
    }

    Ok(())
}
