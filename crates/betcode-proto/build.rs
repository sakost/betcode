//! Build script for betcode-proto
//!
//! Compiles protobuf definitions using tonic-build.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_root = "../../proto";

    let protos = [
        "betcode/v1/common.proto",
        "betcode/v1/agent.proto",
        "betcode/v1/version.proto",
        "betcode/v1/config.proto",
        "betcode/v1/health.proto",
        "betcode/v1/worktree.proto",
    ];

    let proto_paths: Vec<_> = protos
        .iter()
        .map(|p| format!("{}/{}", proto_root, p))
        .collect();

    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(&proto_paths, &[proto_root])?;

    Ok(())
}
