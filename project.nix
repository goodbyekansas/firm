{ nedrylandOverride ? null }:
let
  sources = import ./nix/sources.nix;
  nedryland = (if nedrylandOverride == null then (import sources.nedryland) else nedrylandOverride);
  # Declare our project
  project = nedryland.mkProject {
    name = "firm";
    configFile = ./firm.toml;
    baseExtensions = [
      (import ./extensions/function.nix)
    ];
  };

  protocols = project.declareComponent ./protocols/protocols.nix { };

  typesWithoutServices = project.declareComponent ./utils/rust/firm-types/firm-types.nix {
    protocols = protocols.withoutServices.rust;
  };

  typesWithServices = typesWithoutServices.override {
    protocols = protocols.withServices.rust;
  };

  # declare the components of the project and their dependencies
  components = rec {
    inherit protocols;

    wasiPythonShims = project.declareComponent ./runtimes/python/wasi-python-shims/wasi-python-shims.nix { };
    wasiPython = project.declareComponent ./runtimes/python/wasiPython.nix {
      inherit wasiPythonShims;
      pythonSource = sources.python;
    };

    runtimes = {
      python = project.declareComponent ./runtimes/python/python.nix {
        inherit wasiPython firmRust avery bendini wasiPythonShims;
        declareComponent = project.declareComponent;
        firmTypes = firmTypes.rust;
      };
    };

    avery = project.declareComponent ./services/avery/avery.nix {
      inherit tonicMiddleware;
      types = typesWithServices;
    };

    bendini = project.declareComponent ./clients/bendini/bendini.nix {
      inherit tonicMiddleware;
      types = typesWithServices;
    };

    firmTypes = {
      rust = typesWithoutServices;
      python = project.declareComponent ./utils/python/firm-types/firm-types.nix { protocols = protocols.withoutServices.python; };
    };

    firmRust = project.declareComponent ./utils/rust/firm-rust/firm-rust.nix {
      types = typesWithoutServices;
    };

    osPackaging = project.declareComponent ./deployment/os-packaging.nix {
      linuxPackages = [
        avery
        bendini
      ];

      windowsPackages = [ ];
    };

    quinn = project.declareComponent ./services/quinn/quinn.nix {
      types = typesWithServices;
    };

    tonicMiddleware = project.declareComponent ./utils/rust/tonic-middleware/tonic-middleware.nix {
      protocols = protocols.withServices.rust; # This brings tonic which we will need. A bit hard to see.
    };

  };

  capturedBendiniPackage = components.bendini.package;
  setupFunctionDeployment = { components, endpoint ? "tcp://[::1]", port ? 1939, }: (builtins.mapAttrs
    (
      name:
      comp:
      if comp.isNedrylandComponent or false then
        (comp // (
          if (builtins.hasAttr "deployment" comp) && (builtins.hasAttr "function" comp.deployment) then rec {
            deployment = comp.deployment // {
              function = comp.deployment.function {
                inherit endpoint port;
                bendini = capturedBendiniPackage;
              };
            };
            # TODO: need to re-evaluate the combined "deploy" target here since we changed
            # deployment above
            # we could figure out a way where these shenanigans with injecting the bendini
            # component into the deployment is not necessary (maybe callPackage pattern for
            # components?)
            deploy = project.mkCombinedDeployment "${comp.package.name}-deploy" deployment;
          } else { }
        )) else
        (
          if builtins.isAttrs comp then
            (
              setupFunctionDeployment { components = comp; inherit endpoint port; }
            ) else comp
        )
    )
    (components)
  );
in
# create the build grid (accessed with nix-build, exposed through default.nix)
project.mkGrid {
  inherit components;

  # create the project shells (accessed with nix-shell, exposed through shell.nix)
  extraShells = { };

  # Add functions and other things you want to re-export, making it publicly visible to users of firm.
  lib = {
    inherit setupFunctionDeployment;
  };
}
