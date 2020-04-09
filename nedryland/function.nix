{ base, pkgs }:
let
  deployFunction = { package }: {
    type = "function";
    derivation = { lomax, endpoint, port }: pkgs.stdenv.mkDerivation {
      name = "deploy-${package.name}";
      inputPackage = package;
      inherit lomax;
      builder = builtins.toFile "builder.sh" ''
        source $stdenv/setup
        mkdir -p $out
        $lomax/bin/lomax --address ${endpoint} --port ${builtins.toString port} register $inputPackage/${package.code} $inputPackage/manifest.toml 2>&1 | tee $out/command-output
      '';
    };
  };
  mkFunction = { name, package, manifest, code }:
    let
      packageWithManifest = package.overrideAttrs (
        oldAttrs: {
          installPhase = ''
            ${oldAttrs.installPhase}
            cp ${manifest } $out/manifest.toml
          '';
          inherit code;
        }
      );
    in
      base.mkComponent {
        package = packageWithManifest;
        deployment = {
          function = deployFunction { package = packageWithManifest; };
        };
      };
in
base.extend.mkExtension {
  componentTypes = base.extend.mkComponentType {
    name = "function";
    createFunction = mkFunction;
  };
  deployFunctions = {
    inherit deployFunction;
  };
  languages = base.extend.mkLanguageHelper {
    language = "rust";
    functions = {
      mkFunction = attrs@{ name, src, manifest, buildInputs ? [], extensions ? [], targets ? [], ... }:
        let
          component = base.languages.rust.mkComponent (
            attrs // {
              targets = targets ++ [ "wasm32-wasi" ];
              hasTests = false;
              defaultTarget = "wasm32-wasi";
            }
          );
          newPackage = component.package.overrideAttrs (
            oldAttrs: {
              installPhase = ''
                ${oldAttrs.installPhase}
                mkdir -p $out/bin
                cp target/wasm32-wasi/release/*.wasm $out/bin
              '';
            }
          );
        in
          mkFunction { inherit manifest name; package = newPackage; code = "bin/${newPackage.name}.wasm"; };
    };
  };
}
