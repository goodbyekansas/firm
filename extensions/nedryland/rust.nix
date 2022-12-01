oxalica:
{ base, pkgsCross, callPackage, runCommand }:
let
  rust = base.languages.rust;

  nightlyToolset = pkgs:
    let
      rustOverlay = oxalica (pkgs // rustOverlay) pkgs;

      rustNightly = rustOverlay.rust-bin.nightly."2022-11-29".default.override {
        extensions = [ "rust-analyzer" "rust-src" ];
        targets = [ "wasm32-wasi" ];
      };
    in
    base.languages.rust.mkRustToolset {
      rustc = rustNightly;
      cargo = rustNightly;
      clippy = rustNightly;
      rust-analyzer = rustNightly;
      rustfmt = rustNightly;
    };

  nightlyWasiToolset = nightlyToolset pkgsCross.wasi32.buildPackages;
in
{
  languages.rust.nativeTools = rust.override {
    crossTargets = {
      windows = rust.mkCrossTarget {
        name = "Microsoft® Windows";
        pkgs = pkgsCross.mingwW64;
        attrs = pkgAttrs:
          rec {
            doCrossCheck = pkgAttrs.doCrossCheck or true;
            runner = callPackage ./windows-runner.nix { };

            nativeBuildInputs = rust.combineInputs
              pkgAttrs.nativeBuildInputs or [ ]
              [ runner ];

            buildInputs = rust.combineInputs
              pkgAttrs.buildInputs or [ ]
              (pkgs: [ pkgs.windows.pthreads ]);
          };
      };
    };
  };

  languages.rust.withWasi = rust.override {
    crossTargets.wasi = rust.mkCrossTarget {
      name = "wasi";
      pkgs = pkgsCross.wasi32;
      attrs = pkgAttrs:
        rec {
          doCrossCheck = pkgAttrs.doCrossCheck or true;
          runner = callPackage ./wasi-runner.nix { };

          nativeBuildInputs = rust.combineInputs
            pkgAttrs.nativeBuildInputs or [ ]
            [ runner ];
        };

      rustToolset = rust.mkRustToolset rec {
        inherit (pkgsCross.wasi32.buildPackages) rust-analyzer rustfmt;
        rustc = (pkgsCross.wasi32.buildPackages.rustc.override {
          rust = pkgsCross.wasi32.buildPackages.rust // {
            inherit (base.languages.rust) toRustTarget;
            toRustTargetSpec = base.languages.rust.toRustTarget;
          };
        }).overrideAttrs (
          let
            # rust expects this to be here
            wasilibc = runCommand
              "linked-waslibc"
              { crossLib = pkgsCross.wasi32.wasilibc; }
              ''
                mkdir -p $out/lib
                ln -s $crossLib/lib $out/lib/wasm32-wasi
              '';
          in
          attrs:
          {
            configureFlags = (builtins.filter (flag: flag != "--enable-profiler") attrs.configureFlags) ++ [
              ''--set=target.wasm32-wasi.wasi-root=${wasilibc}''
              ''--disable-docs''
            ];
          }
        );

        cargo = pkgsCross.wasi32.buildPackages.cargo.override {
          inherit rustc;
        };
        clippy = pkgsCross.wasi32.buildPackages.clippy.override {
          inherit rustc;
        };
      };
    };
  };

  languages.rust.nightly = (rust.override nightlyWasiToolset) // {
    withWasi = rust.override (nightlyWasiToolset // {
      crossTargets.wasi = rust.mkCrossTarget {
        name = "wasi";
        pkgs = pkgsCross.wasi32;
        attrs = pkgAttrs:
          rec {
            doCrossCheck = pkgAttrs.doCrossCheck or true;
            runner = callPackage ./wasi-runner.nix { };

            nativeBuildInputs = rust.combineInputs
              pkgAttrs.nativeBuildInputs or [ ]
              [ runner ];
          };
        rustToolset = nightlyWasiToolset;
      };
    });
  };
}
