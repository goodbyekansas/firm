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
      manifestContent = if builtins.isPath manifest then (builtins.fromTOML (builtins.readFile manifest)) else manifest;
      manifestWithChecksum = manifestContent // {
        checksums = {
          sha256 = "$SHA256";
        };
        # TODO: In the future we must support signatures for functions as well
      };
      packageWithManifest = package.overrideAttrs (
        oldAttrs: {
          buildInputs = oldAttrs.buildInputs or [] ++ [ pkgs.utillinux ];
          manifestContent = builtins.toJSON manifestWithChecksum;
          passAsFile = oldAttrs.passAsFile or [] ++ [ "manifestContent" ];
          postInstall = ''
            ${oldAttrs.postInstall or ""}
            if [ -f $out/${code} ]; then
              cat $manifestContentPath | \
              SHA256=$(sha256sum $out/${code} | cut -d " " -f 1) ${pkgs.envsubst}/bin/envsubst | \
              ${pkgs.remarshal}/bin/json2toml -o $out/manifest.toml
            else
              echo "ERROR: specified code does not exist..."
              exit 1
            fi
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
