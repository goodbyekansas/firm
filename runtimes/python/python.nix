{ base, pkgs, wasiPython }:
base.mkComponent {
  package = pkgs.pkgsCross.wasi32.clang11Stdenv.mkDerivation {
    name = "python-runtime";
    src = ./.;
    nativeBuildInputs = [ pkgs.wasmtime ];

    installPhase = ''
      mkdir -p $out

      touch $out/there-will-be-a.wasm

      echo "TODO"
    '';
  };
}
