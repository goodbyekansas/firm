use crate::proto::execute_response::Result as ProtoResult;

pub trait FunctionExecutor {
    fn execute(&self, code: &[u8]) -> ProtoResult;
}

#[derive(Default, Debug)]
struct MayaExecutor {}

impl FunctionExecutor for MayaExecutor {
    fn execute(&self, _code: &[u8]) -> ProtoResult {
        ProtoResult::Ok("hello, world!".to_owned())
    }
}

#[derive(Default, Debug)]
struct WasmExecutor {}

impl FunctionExecutor for WasmExecutor {
    fn execute(&self, _code: &[u8]) -> ProtoResult {
        ProtoResult::Ok("".to_owned())
    }
}

pub fn lookup_executor(name: &str) -> Result<Box<dyn FunctionExecutor>, String> {
    match name {
        "maya" => Ok(Box::new(MayaExecutor {})),

        "wasm" => Ok(Box::new(WasmExecutor {})),

        ee => Err(format!("Failed to find execution environment: {}", ee)),
    }
}
