use std::{env, path::Path};

fn main() {
    let proto_env = env::var("PROTOBUF_DEFINITIONS_LOCATION").unwrap();
    let proto_path = Path::new(&proto_env);
    tonic_build::configure()
        .build_server(false)
        .compile(
            &[format!("{}/functions.proto", proto_path.to_str().unwrap())],
            &[proto_path.to_str().unwrap().to_owned()],
        )
        .map_err(|e| format!("failed to compile protobuf: {}", e))
        .unwrap();
}
