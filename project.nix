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

    firmRust = project.declareComponent ./utils/rust/firm-rust/firm-rust.nix {
      protocols = protocols.rust.onlyMessages;
    };
    tonicMiddleware = project.declareComponent ./utils/rust/tonic-middleware/tonic-middleware.nix {
      protocols = protocols.rust.withServices; # This brings tonic which we will need. A bit hard to see.
    };
    avery = project.declareComponent ./services/avery/avery.nix {
      protocols = protocols.rust.withServices;
      protocolsTestHelpers = protocols.rust.testHelpers { protocols = protocols.rust.withServices; };
    };
    bendini = project.declareComponent ./clients/bendini/bendini.nix {
      inherit tonicMiddleware;
      protocols = protocols.rust.withServices;
      protocolsTestHelpers = protocols.rust.testHelpers { protocols = protocols.rust.withServices; };
    };

    osPackaging = project.declareComponent ./deployment/os-packaging.nix {
      linuxPackages = [
        avery
        bendini
      ];

      windowsPackages = [ ];
    };

    quinn = project.declareComponent ./services/quinn/quinn.nix {
      protocols = protocols.rust.withServices;
      protocolsTestHelpers = protocols.rust.testHelpers { protocols = protocols.rust.withServices; };
    };
  };
  capturedBendiniPackage = components.bendini.package;
  # TODO credentials must be removed. Need to have a local auth service for that.
  setupFunctionDeployment = { components, endpoint ? "tcp://[::1]", port ? 1939, credentials ? "" }: (builtins.mapAttrs
    (
      name:
      comp:
      comp // (
        if (builtins.hasAttr "deployment" comp) && (builtins.hasAttr "function" comp.deployment) then {
          deployment = comp.deployment // {
            function = comp.deployment.function {
              inherit endpoint port credentials;
              bendini = capturedBendiniPackage;
              local = endpoint == "tcp://[::1]";
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
  deploy = { };

  # create the project shells (accessed with nix-shell, exposed through shell.nix)
  extraShells = { };

  # Add functions and other things you want to re-export, making it publicly visible to users of firm.
  lib = {
    inherit setupFunctionDeployment;
  };
}
