{ base, pkgs, firmRust, firmTypes, avery, bendini, wasiPython, runCommand, wasiPythonShims }:
let
  examples = {
    hello = base.callFile ./examples/hello/hello.nix { };
    firmApi = base.callFile ./examples/firm-api/firm-api.nix { };
    firmApiError = base.callFile ./examples/firm-api/firm-api-error.nix { };
    networking = base.callFile ./examples/networking/networking.nix { };
    yamler = base.callFile ./examples/yamler/yamler.nix { };
  };

  fileSystemImage = (pkgs.linkFarm "python-runner-fs-image" [{
    name = "lib";
    path = "${wasiPython.package}/lib/python3.8";
  }]);

  zlib = pkgs.pkgsCross.wasi32.zlib.override {
    stdenv = pkgs.pkgsCross.wasi32.clang11Stdenv;
  };

in
base.mkRuntime {
  inherit examples fileSystemImage;

  name = "python-runtime";
  runtimeName = "python";
  src = ./.;

  testFeatures = [ "mock" ];
  doCrossCheck = true;

  nativeBuildInputs = [ pkgs.python3 ];

  buildInputs = [ firmRust.package firmTypes.package wasiPythonShims.package zlib ];
  shellInputs = [ pkgs.netcat-gnu ];
  shellHook = ''
    runApiExample()
    {
      echo "Running firmApi example"
      command cargo run firmApi \
      -i str_input="hej" \
      -i int_input=5 \
      -i float_input=3.15 \
      -i bool_input=false \
      -i bytes_input=32 \
      -i str_list_input="[heffaklump 🐙]" \
      -i int_list_input="[1 -14 4444]" \
      -i bool_list_input="[true false true false]" \
      -i float_list_input="[0.1 0.2 123123123.2]" | sed "s/^/  [firmApi] /"

      echo "Running firmApiError example"
      command cargo run firmApiError | sed "s/^/  [firmApiError] /"
    }

    runNetExample() {
      echo "Starting netcat on port 3333..."
      nc -l -k -p 3333 | sed "s/^/[network] /" &
      command cargo run networking -i port=3333
      kill %1 && wait %1
    }

    runDepsExample() {
      echo "Running yaml dependency example"
      command cargo run yamler \
      -i yaml="$1" \
      -i yamlkey="$2"
    }

  '';
  RUSTFLAGS = "-Ctarget-feature=-crt-static -Clink-args=-lwasi-emulated-signal -lz";

  # Environment variable used to skip a step in the build process for wasi shims.
  # We build directly with the library so we do not need any C bindgens.
  # Setting it to anything will skip it.
  WASI_PYTHON_SHIMS_SKIP_C_BINDGEN = "Yes";

  PYO3_CROSS_LIB_DIR = "${wasiPython.package}/lib/";
}
