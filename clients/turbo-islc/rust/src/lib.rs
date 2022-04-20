use std::{
    fmt::Display,
    process::Stdio,
    str::{self},
};

use syn::parse::Parse;

enum Target {
    Wasm,
    Wasmtime,
}

impl Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Target::Wasm => "wasm",
            Target::Wasmtime => "wasmtime",
        })
    }
}

struct Opts {
    input: String,
    target: Target,
}

struct Syntax {
    input: syn::LitStr,
    _arrow: syn::Token!(->),
    target: syn::Ident,
}

impl Parse for Opts {
    fn parse(stream: syn::parse::ParseStream) -> syn::Result<Self> {
        let syntax = Syntax {
            input: stream.parse().unwrap(),
            _arrow: stream.parse().unwrap(),
            target: stream.parse().unwrap(),
        };

        Ok(Self {
            input: syntax.input.value(),
            target: match syntax.target.to_string().as_str() {
                "wasm" => Target::Wasm,
                _ => Target::Wasmtime,
            },
        })
    }
}

#[proc_macro]
pub fn turbo(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let args = syn::parse_macro_input!(input as Opts);

    match std::process::Command::new("turbo")
        .args([args.input, format!("--target=rust-{}", args.target)])
        .stderr(Stdio::piped())
        .output()
    {
        Ok(output) => {
            if output.status.success() {
                str::from_utf8(output.stderr.as_slice())
                    .unwrap_or_else(|e| panic!("Turbo made non-utf8 output: {}", e))
                    .parse()
                    .unwrap()
            } else {
                panic!(
                    "Turbo exited with status {}: stdout: {} stderr: {}",
                    output.status,
                    str::from_utf8(output.stdout.as_slice()).unwrap(),
                    str::from_utf8(output.stderr.as_slice()).unwrap()
                )
            }
        }
        Err(e) => panic!("Failed to run turbo: {}", e),
    }
}
