{ base, pkgs, firmRust, firmTypes, wasiPython, avery, bendini, declareComponent, runCommand, wasiPythonShims }:
let
  examples = {
    hello = declareComponent ./examples/hello/hello.nix { };
  };

  env = (builtins.mapAttrs (n: v: (v.deployment.function { bendini = bendini.package; })) examples);
  envVars = pkgs.writeTextFile {
    name = "runtime-runner-env";
    text = builtins.foldl'
      (acc: curr: ''
        ${acc}
        declare -x ${curr}="${builtins.replaceStrings [ "$" "\"" ] [ "\\$" "\\\"" ] (builtins.toString (builtins.getAttr curr env))}"
      '')
      ""
      (builtins.attrNames env);
  };

  runner = { name, fileSystemImage ? null }: runCommand "runtime-runner-python.bash"
    {
      inherit fileSystemImage name;
      preferLocalBuild = true;
      allowSubstitutes = false;
      functions = envVars;
    }
    ''
      substituteAll ${./runtime-runner.bash} $out
      chmod +x $out
    '';

  fileSystemImage = (pkgs.linkFarm "python-runner-fs-image" [{
    name = "lib";
    path = "${wasiPython.package}/lib/python3.8";
  }]);

  mkPackage = base.languages.rust.mkPackage.override { stdenv = pkgs.pkgsCross.wasi32.clang11Stdenv; };

in
base.mkComponent {
  name = "python-runtime";
  inherit examples;
  package = (mkPackage {
    name = "python-runtime";
    runtimeName = "python";
    src = ./.;

    testFeatures = [ "mock" ];

    targets = [ "wasm32-wasi" ];
    defaultTarget = "wasm32-wasi";

    doCrossCheck = true;

    nativeBuildInputs = [ pkgs.python3 pkgs.wasmtime ]
      ++ pkgs.lib.optional pkgs.stdenv.isDarwin pkgs.llvmPackages_11.llvm;

    buildInputs = [ firmRust.package firmTypes.package wasiPythonShims.package ];
    shellInputs = [ pkgs.coreutils bendini.package avery.package pkgs.wabt ];
    RUSTFLAGS = "-Ctarget-feature=-crt-static -Clinker-flavor=gcc -Clink-args=-lwasi-emulated-signal";
    CARGO_TARGET_WASM32_WASI_RUNNER = runner {
      name = "python";
      inherit fileSystemImage;
    };

    # Environment variable used to skip a step in the build process for wasi shims.
    # We build directly with the library so we do not need any C bindgens.
    # Setting it to anything will skip it.
    WASI_PYTHON_SHIMS_SKIP_C_BINDGEN = "Yes";

    # needed for pyo3, not technically correct
    CARGO_CFG_TARGET_FAMILY = "unix";
    PYO3_CROSS_LIB_DIR = "${wasiPython.package}/lib/";

    useNightly = "2021-01-27";
  }).overrideAttrs (oldAttrs: {
    installPhase = ''
      mkdir -p $out/share/avery/runtimes

      # this is if filesystem image is not provided when factoring out:
      # cp target/wasm32-wasi/release/*.wasm $out/share/avery/runtimes/${oldAttrs.runtimeName}.wasm
      # else:
      ln -s ${fileSystemImage} fs

      # -h to resolve symlinks
      # also set mode because of https://github.com/alexcrichton/tar-rs/issues/242
      echo "📦 creating tar file for runtime filesystem image..."
      tar -chzf "$out/share/avery/runtimes/${oldAttrs.runtimeName}.tar.gz" --mode='a+rwX' fs -C target/wasm32-wasi/release --owner 0 --group 0 ${oldAttrs.runtimeName}.wasm
      echo "🌅 Image created!"
    '';
  });
}
