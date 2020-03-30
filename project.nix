let
  # the if statement here is to be able to switch to
  # a local dev version of nedryland
  # if you do not need this, just keep the else statement
  nedryland = import (
    if builtins.getEnv "NEDRYLAND_PATH" != "" then
      (./. + "/${builtins.getEnv "NEDRYLAND_PATH"}")
    else
      builtins.fetchGit {
        name = "nedryland";
        url = "git@github.com:goodbyekansas/nedryland.git";
        rev = "0b380506fd684cc4204823a54df1ab39035c61b1";
      }
  );

  # declare our project
  project = nedryland.mkProject {
    name = "firm";
    configFile = ./firm.toml;
    protoLocation = ./protocols;
    baseExtensions = [
      (import ./nedryland/function.nix)
    ];
  };

  # declare the components of the project and their dependencies
  components = rec {
    start-maya = project.declareComponent ./functions/start-maya/start-maya.nix {};
    avery = project.declareComponent ./services/avery/avery.nix {
      inputFunctions = [ start-maya ];
    };
    bendini = project.declareComponent ./clients/bendini/bendini.nix {};
    lomax = project.declareComponent ./clients/lomax/lomax.nix {
      inherit avery;
    };

    os-packaging = project.declareComponent ./deployment/os-packaging.nix {
      linuxPackages = [
        avery
      ];

      windowsPackages = [];
    };
  };
  getFunctionDeployments = { components, endpoint ? "tcp://[::1]", port ? 1939 }: builtins.map (
    drv:
      drv {
        inherit endpoint port;
        lomax = components.lomax.package;
      }
  ) (
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
  extraShells = {};

  # Add functions and other things you want to re-export, making it publicly visible to users of firm.
  lib = {
    inherit getFunctionDeployments;
  };
}
