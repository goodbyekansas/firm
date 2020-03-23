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
        ref = "41bfa2daf29fdaa357b10bedde09ec6659b37ddd";
      }
  );

  # declare our project
  project = nedryland.mkProject {
    name = "firm";
    configFile = ./firm.json;
    protoLocation = ./protocols;
  };

  # declare the components of the project and their dependencies
  components = rec {
    start-maya = project.declareComponent ./functions/start-maya/start-maya.nix {};
    avery = project.declareComponent ./services/avery/avery.nix {
      dependencies = {
        inputFunctions = [ start-maya ];
      };
    };
    bendini = project.declareComponent ./clients/bendini/bendini.nix {};
    lomax = project.declareComponent ./clients/lomax/lomax.nix {};

    os-packaging = project.declareComponent ./deployment/os-packaging.nix {
      dependencies = {
        linuxPackages = [
          avery
        ];

        windowsPackages = [];
      };
    };
  };
in
{
  # create the build grid (accessed with nix-build, exposed through default.nix)
  grid = project.mkGrid {
    inherit components;
  };


  # create the project shells (accessed with nix-shell, exposed through shell.nix)
  shells = project.mkShells {
    inherit components;
    extraShells = {};
  };
}
