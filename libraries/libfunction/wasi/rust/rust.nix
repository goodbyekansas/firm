{ base, wabt, wasmtime }:

(base.languages.rust.mkLibrary {
  name = "libfunction-rust";
  src = ./.;
  defaultTarget = "wasi";
  shellInputs = [ wabt ];
  checkInputs = [ wasmtime ];

  doCrossCheck = true;
})
