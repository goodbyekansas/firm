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

  # declare the components of the project and their dependencies
  components = rec {
    inherit protocols;

    wasiFunctionUtils = project.declareComponent ./utils/rust/wasi-function-utils/wasi-function-utils.nix {
      protocols = protocols.rust.onlyMessages;
    };
    avery = project.declareComponent ./services/avery/avery.nix {
      protocols = protocols.rust.withServices;
      protocolsTestHelpers = protocols.rust.testHelpers { protocols = protocols.rust.withServices; };
    };
    bendini = project.declareComponent ./clients/bendini/bendini.nix {
      protocols = protocols.rust.withServices;
    };
    lomax = project.declareComponent ./clients/lomax/lomax.nix {
      protocols = protocols.rust.withServices;
      protocolsTestHelpers = protocols.rust.testHelpers { protocols = protocols.rust.withServices; };
    };

    osPackaging = project.declareComponent ./deployment/os-packaging.nix {
      linuxPackages = [
        avery
        bendini
        lomax
      ];

      windowsPackages = [ ];
    };

    quinn = project.declareComponent ./services/quinn/quinn.nix {
      protocols = protocols.rust.withServices;
      protocolsTestHelpers = protocols.rust.testHelpers { protocols = protocols.rust.withServices; };
    };
  };
  capturedLomaxPackage = components.lomax.package;

  setupFunctionDeployment = { components, endpoint ? "tcp://[::1]", port ? 1939 }: (builtins.mapAttrs
    (
      name:
      comp:
      comp // (
        if (builtins.hasAttr "deployment" comp) && (builtins.hasAttr "function" comp.deployment) then {
          deployment = comp.deployment // {
            function = comp.deployment.function {
              inherit endpoint port;
              lomax = capturedLomaxPackage;
            };
          };
        } else { }
      )
    )
    (components)
  );
in
# create the build grid (accessed with nix-build, exposed through default.nix)
project.mkGrid {
  inherit components;
  deploy = rec {
    local = [ ];
    prod = [ ];
  };

  # create the project shells (accessed with nix-shell, exposed through shell.nix)
  extraShells = { };

  # Add functions and other things you want to re-export, making it publicly visible to users of firm.
  lib = {
    inherit setupFunctionDeployment;
  };
}
