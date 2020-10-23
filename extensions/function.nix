{ base, pkgs }:
let
  deployFunction = { package }:
    # TODO credentials must be removed. Need to have a local auth service for that.
    { lomax, endpoint, port, credentials, local ? false }: pkgs.stdenv.mkDerivation ({
      name = "deploy-${package.name}";
      inputPackage = package;
      inherit lomax;
      preferLocalBuild = local;
      SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
      builder = builtins.toFile "builder.sh" ''
        source $stdenv/setup
        mkdir -p $out
        $lomax/bin/lomax --address ${endpoint} --port ${builtins.toString port} register $inputPackage/manifest.toml 2>&1 | tee $out/command-output
      '';
    } // (if credentials != "" then { OAUTH_TOKEN = credentials; } else { }));

  mkFunction = attrs@{ name, package, manifest, code, deploy ? true, ... }:
    let
      manifestGenerator = pkgs.callPackage ./manifest.nix {
        inherit name;
        manifest = manifest // {
          code = {
            path = code;
          };
        };
      };

      packageWithManifest = package.overrideAttrs (oldAttrs: {
        nativeBuildInputs = oldAttrs.nativeBuildInputs or [ ] ++ [ manifestGenerator ];
      });
    in
    base.mkComponent (
      attrs // {
        package = packageWithManifest;
      } // (
        if deploy then {
          deployment = {
            function = deployFunction { package = packageWithManifest; };
          };
        } else { }
      )
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
        , ...
        }:
        let
          package = base.languages.rust.mkPackage {
            inherit src name rustDependencies useNightly buildInputs extraChecks buildFeatures testFeatures;
            targets = targets ++ [ "wasm32-wasi" ];
            defaultTarget = "wasm32-wasi";
            shellInputs = attrs.shellInputs or [ ];
            shellHook = attrs.shellHook or "";
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
