{ base, pkgs }:
let
  deployFunction = { package }:
    { lomax, endpoint, port }: pkgs.stdenv.mkDerivation {
      name = "deploy-${package.name}";
      inputPackage = package;
      inherit lomax;
      builder = builtins.toFile "builder.sh" ''
        source $stdenv/setup
        mkdir -p $out
        $lomax/bin/lomax --address ${endpoint} --port ${builtins.toString port} register $inputPackage/manifest.toml 2>&1 | tee $out/command-output
      '';
    };

  # TODO investigate if code should be different from attachment, i.e: code vs manifest.attachments
  mkFunction = attrs@{ name, package, manifest, code, ... }:
    let
      manifestGenerator = pkgs.callPackage ./manifest.nix {
        inherit name code manifest;
        attachments = manifest.attachments or { };
      };

      packageWithManifest = package.overrideAttrs (oldAttrs: {
        buildInputs = oldAttrs.buildInputs ++ [ manifestGenerator ];
      });
    in
    base.mkComponent (
      attrs // {
        package = packageWithManifest;
        deployment = {
          function = deployFunction { package = packageWithManifest; };
        };
      }
    );
in
base.extend.mkExtension {
  componentTypes = base.extend.mkComponentType {
    name = "function";
    createFunction = mkFunction;
  };
  deployFunctions = {
    inherit deployFunction;
  };
  languages = {
    rust = {
      mkFunction =
        attrs@{ name
        , src
        , manifest
        , buildInputs ? [ ]
        , extensions ? [ ]
        , targets ? [ ]
        , rustDependencies ? [ ]
        , useNightly ? ""
        , extraChecks ? ""
        , buildFeatures ? [ ]
        , testFeatures ? [ ]
        }:
        let
          package = base.languages.rust.mkPackage {
            inherit src name rustDependencies useNightly buildInputs extraChecks buildFeatures testFeatures;
            targets = targets ++ [ "wasm32-wasi" ];
            defaultTarget = "wasm32-wasi";
          };

          newPackage = package.overrideAttrs (
            oldAttrs: {
              installPhase = ''
                ${oldAttrs.installPhase}
                mkdir -p $out/bin
                cp target/wasm32-wasi/release/*.wasm $out/bin
              '';
            }
          );
        in
        mkFunction (attrs // {
          package = newPackage;
          code = "bin/${newPackage.name}.wasm";
        });
    };
    python = { };
  };
}
