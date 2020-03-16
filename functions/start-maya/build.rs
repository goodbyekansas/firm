use std::{env, path::Path};

fn main() {
    let proto_env = env::var("PROTOBUF_DEFINITIONS_LOCATION").unwrap();
    let proto_path = Path::new(&proto_env);

    prost_build::compile_protos(
        &[format!("{}/functions.proto", proto_path.to_str().unwrap())],
        &[proto_path.to_str().unwrap().to_owned()],
    )
    .unwrap();
}
