let
  # the if statement here is to be able to switch to
  # a local dev version of nedryland
  # if you do not need this, just keep the else statement
  nedryland =
    import (
      if builtins.getEnv "NEDRYLAND_PATH" != "" then
        (builtins.getEnv "PWD" + "/${builtins.getEnv "NEDRYLAND_PATH"}")
      else
        builtins.fetchGit {
          name = "nedryland";
          url = "git@github.com:goodbyekansas/nedryland.git";
          rev = "f6c321eb9280debe9f670661fc8ace4bddbfb2eb";
        }
    );

  # declare our project
  project = nedryland.mkProject {
    name = "firm";
    configFile = ./firm.toml;
    baseExtensions = [
      (import ./nedryland/function.nix)
    ];
  };
  protocols = import ./protocols project;

  # declare the components of the project and their dependencies
  components = rec {
    inherit protocols;

    wasiFunctionUtils = project.declareComponent ./utils/rust/gbk/gbk.nix {
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
  };
  capturedLomaxPackage = components.lomax.package;
  getFunctionDeployments = { components, endpoint ? "tcp://[::1]", port ? 1939 }: builtins.map
    (
      drv:
      drv {
        inherit endpoint port;
        lomax = capturedLomaxPackage;
      }
    )
    (
      nedryland.getDeployments { inherit components; type = "function"; }
    );
in
# create the build grid (accessed with nix-build, exposed through default.nix)
project.mkGrid {
  inherit components;
  deploy = rec {
    functions = getFunctionDeployments {
      inherit components;
    };

    local = [
      functions
    ];

    prod = [
      (
        getFunctionDeployments {
          inherit components;
          endpoint = "tcp://a.production.registry";
          port = 1337;
        }
      )
    ];
  };

  # create the project shells (accessed with nix-shell, exposed through shell.nix)
  extraShells = { };

  # Add functions and other things you want to re-export, making it publicly visible to users of firm.
  lib = {
    inherit getFunctionDeployments;
  };
}
