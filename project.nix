let
  sources = import ./nix/sources.nix;
  nedryland = import sources.nedryland;

  # Declare our project
  project = nedryland.mkProject {
    name = "firm";
    configFile = ./firm.toml;
    baseExtensions = [
      (import ./extensions/function.nix)
    ];
  };

  protocols = import ./protocols project;

  typesWithoutServices = project.declareComponent ./utils/rust/firm-types/firm-types.nix {
    protocols = protocols.rust.onlyMessages;
  };

  typesWithServices = typesWithoutServices.override {
    protocols = protocols.rust.withServices;
  };

  # declare the components of the project and their dependencies
  components = rec {
    inherit protocols;

    avery = project.declareComponent ./services/avery/avery.nix {
      types = typesWithServices;
    };

    bendini = project.declareComponent ./clients/bendini/bendini.nix {
      inherit tonicMiddleware;
      types = typesWithServices;
    };

    firmTypes = {
      rust = typesWithoutServices;
      python = project.declareComponent ./utils/python/firm-types/firm-types.nix { protocols = protocols.python; };
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
      protocols = protocols.rust.withServices; # This brings tonic which we will need. A bit hard to see.
    };

  };

  capturedBendiniPackage = components.bendini.package;

  # TODO credentials must be removed. Need to have a local auth service for that.
  setupFunctionDeployment = { components, endpoint ? "tcp://[::1]", port ? 1939, credentials ? "" }: (builtins.mapAttrs
    (
      name:
      comp:
      if comp.isNedrylandComponent or false then
        (comp // (
          if (builtins.hasAttr "deployment" comp) && (builtins.hasAttr "function" comp.deployment) then {
            deployment = comp.deployment // {
              function = comp.deployment.function {
                inherit endpoint port credentials;
                bendini = capturedBendiniPackage;
                local = endpoint == "tcp://[::1]";
              };
            };
          } else { }
        )) else setupFunctionDeployment { components = comp; inherit endpoint port credentials; }
    )
    (components)
  );
in
# create the build grid (accessed with nix-build, exposed through default.nix)
project.mkGrid {
  inherit components;
  deploy = { };

  # create the project shells (accessed with nix-shell, exposed through shell.nix)
  extraShells = { };

  # Add functions and other things you want to re-export, making it publicly visible to users of firm.
  lib = {
    inherit setupFunctionDeployment;
  };
}
