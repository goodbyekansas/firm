{ base, pkgs }:
let
  deployFunction = { package }:
    { bendini, endpoint, port }: base.deployment.mkDeployment {
      name = "deploy-${package.name}";
      preDeploy = "";
      postDeploy = "";
      deployPhase = ''
        SSL_CERT_FILE="${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt" ${bendini}/bin/bendini \
        --address ${endpoint} \
        --port ${builtins.toString port} \
        register ${package}/manifest.toml
      '';
    };

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
        { name
        , src
        , manifest
        , extensions ? [ ]
        , targets ? [ ]
        , useNightly ? ""
        , extraChecks ? ""
        , buildFeatures ? [ ]
        , testFeatures ? [ ]
        , packageAttrs ? { }
        , componentAttrs ? { }
        }:
        let
          package = base.languages.rust.mkPackage (packageAttrs // {
            inherit src name useNightly extraChecks buildFeatures testFeatures;
            targets = targets ++ [ "wasm32-wasi" ];
            defaultTarget = "wasm32-wasi";
          });

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
        mkFunction (componentAttrs // {
          inherit name manifest;
          package = newPackage;
          code = "bin/${newPackage.name}.wasm";
        });
    };
    python = { };
  };
}
