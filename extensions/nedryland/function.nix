{ base, pkgs, bendini }:
let
  deployFunction = { package }:
    { endpoint ? "tcp://[::1]", port ? 1939 }: base.deployment.mkDeployment {
      name = "deploy-${package.name}";
      preDeploy = "";
      postDeploy = "";
      deployPhase = ''
        SSL_CERT_FILE="${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt" ${bendini.package}/bin/bendini \
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

      packageWithManifest = package.overrideAttrs (oldAttrs:
        (if builtins.elem "installPhase" oldAttrs.phases then {
          nativeBuildInputs = oldAttrs.nativeBuildInputs or [ ] ++ [ manifestGenerator ];
          installPhase = ''
            ${oldAttrs.installPhase or ""}
            generateManifest
          '';
        }
        else
          builtins.abort "\"installPhase\" needs to be in \"phases\" for function manifest generation to work"
        )
      );
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
        , fullCrossCompile ? false
        }:
        let
          mkPackage = (
            if fullCrossCompile then
              base.languages.rust.mkPackage.override { stdenv = pkgs.pkgsCross.wasi32.clang11Stdenv; }
            else
              base.languages.rust.mkPackage
          );
          package = mkPackage (packageAttrs // {
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
    python = {
      mkFunction =
        { name
        , src
        , version
        , packageAttrs ? { }
        , componentAttrs ? { }
        , entrypoint ? "main:main"
        , inputs ? { }
        , outputs ? { }
        , metadata ? { }
        , attachments ? { }
        , dependencies ? (_: [ ])
        }:
        let
          pythonWasiPkgs = pkgs.callPackage ./wasi-python-packages.nix { };
          functionDependencies = dependencies pythonWasiPkgs;

          packagedPythonDependencies = pkgs.stdenv.mkDerivation {
            name = "${name}-dependencies.tar.gz";
            phases = [ "buildPhase" "installPhase" ];

            inherit functionDependencies;

            buildPhase = ''
              mkdir -p dependencies
              for dep in $functionDependencies; do
                cp $dep/lib/wasi-wheels/*.whl dependencies/
                if [ -f $dep/firm/wasi-dependencies ]; then
                  for pd in $(cat "$dep/firm/wasi-dependencies"); do
                    cp $pd/lib/wasi-wheels/*.whl dependencies/
                  done
                fi
              done
            '';

            installPhase = ''
              tar czf $out dependencies
            '';
          };

          package = pkgs.stdenv.mkDerivation {
            inherit name version;

            src = if pkgs.lib.isStorePath src then src else (builtins.path { path = src; inherit name; });

            phases = [ "unpackPhase" "installPhase" ];
            nativeBuildInputs = [ pkgs.python38.pkgs.setuptools ];

            installPhase = ''
              mkdir $out
              ${pkgs.python38}/bin/python setup.py sdist --dist-dir dist --formats=gztar
              cp dist/*.tar.gz $out/code.tar.gz
            '';
          };
          manifest = {
            inherit name version inputs outputs metadata;
            attachments = attachments // (
              if functionDependencies != [ ] then { dependencies = builtins.toString packagedPythonDependencies; }
              else { }
            );
            runtime = { type = "python"; inherit entrypoint; };
          };
        in
        mkFunction (componentAttrs // { inherit name package manifest version; code = "code.tar.gz"; });
    };
  };
}
