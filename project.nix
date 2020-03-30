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
        rev = "1bfc6dbb12518e25ca6da4c0df00b2ed137b95b2";
      }
  );

  # declare our project
  project = nedryland.mkProject {
    name = "firm";
    configFile = ./firm.toml;
    protoLocation = ./protocols;
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
in
{
  # create the build grid (accessed with nix-build, exposed through default.nix)
  grid = project.mkGrid {
    inherit components;
    deploy = rec {
      functions = nedryland.getFunctionDeployments {
        inherit components;
        lomax = components.lomax.package;
      };

      local = [
        functions
      ];

      prod = [
        (
          nedryland.getFunctionDeployments {
            inherit components;
            lomax = components.lomax.package;
            endpoint = "tcp://a.production.registry";
            port = 1337;
          }
        )
      ];
    };
  };


  # create the project shells (accessed with nix-shell, exposed through shell.nix)
  shells = project.mkShells {
    inherit components;
    extraShells = {};
  };
}
