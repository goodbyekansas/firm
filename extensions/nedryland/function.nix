{ base, pkgs, bendini }:
let
  deployFunction = package:
    pkgs.lib.makeOverridable
      (
        { host ? null }:
        let
          host' = if host != null then "--host ${host}" else "";
        in
        base.deployment.mkDeployment {
          name = "deploy-${package.name}";
          deployPhase = ''
            bendiniCommand=${bendini.package}/bin/bendini
            if [ -n "$(which bendini)" ] && [ -z "$BENDINI_DEV" ]; then
              bendiniCommand=bendini
            fi
            $bendiniCommand ${host'} register ${package}/manifest.toml
          '';
        }
      )
      { };

  mkFunction = attrs_@{ name, package, manifest, code, deploy ? true, ... }:
    let
      # deploy is a "magic" target on all components
      # so do not override it
      attrs = builtins.removeAttrs attrs_ [ "deploy" ];
      manifestGenerator = pkgs.callPackage ./function/manifest.nix {
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
      { nedrylandType = "function"; } //
      attrs // {
        package = packageWithManifest;
        docs = attrs.docs or { } // {
          api = import ./function/function-doc.nix pkgs base.parseConfig manifest;
        };
      } // (
        if deploy then {
          deployment = {
            function = deployFunction packageWithManifest;
          };
        } else { }
      )
    );
in
base.extend.mkExtension {
  componentTypes = {
    inherit mkFunction;
  };
  deployFunctions = {
    inherit deployFunction;
  };
  languages = {
    rust = {
      mkFunction =
        packageAttrs@{ name
        , manifest
        , deploy ? true
        , ...
        }:
        mkFunction {
          inherit manifest name deploy;
          code = "bin/${name}.wasm";
          package = (base.languages.rust.mkComponent ((builtins.removeAttrs packageAttrs [ "manifest" "deploy" ]) // {
            defaultTarget = "wasi";
            nedrylandType = "function";
            installPhase = ''
              runHook preInstall
              mkdir -p $out/bin
              cp target/wasm32-wasi/release/*.wasm $out/bin
              runHook postInstall
            '';
          })).wasi;
        };
    };
    python = {
      mkFunction =
        { name
        , src
        , version
        , description ? ""
        , componentAttrs ? { }
        , entrypoint ? "main:main"
        , inputs ? { }
        , outputs ? { }
        , metadata ? { }
        , attachments ? { }
        , dependencies ? (_: [ ])
        }:
        let
          pythonVersion = pkgs.python38;
          pythonWasiPkgs = pkgs.callPackage ./function/wasi-python-packages.nix { };
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

          package = (base.languages.python.mkComponent {
            inherit name version pythonVersion src;
            checkInputs = dependencies;
            preDistPhases = [ "generateManifestPhase" ];
            nativeBuildInputs = (p: [ p.setuptools ]);
            format = "custom";

            buildPhase = ''
              echo "exclude setup.cfg" > MANIFEST.in
              python setup.py sdist --dist-dir dist --formats=gztar
            '';

            installPhase = ''
              mkdir $out
              runHook preInstall

              cp dist/*.tar.gz $out/code.tar.gz
              runHook postInstall
            '';
          }).package;
          manifest = {
            inherit name version inputs outputs metadata description;
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
