{
  description = "Firm is a distributed function execution framework.";

  inputs = {
    nedryland.url = github:goodbyekansas/nedryland/8.0.0;
    nedryglot.url = github:goodbyekansas/nedryglot/1.0.0;
    oxalica.url = github:oxalica/rust-overlay;
    pkgs.url = github:NixOS/nixpkgs/nixos-22.05;
    flake-utils.url = github:numtide/flake-utils;
  };

  outputs =
    { nedryland
    , nedryglot
    , pkgs
    , oxalica
    , flake-utils
    , ...
    }:
    flake-utils.lib.eachDefaultSystem (system:
    let
      pkgs' = pkgs.legacyPackages."${system}";

      projectFn = import ./project.nix;
      project = projectFn {
        pkgs = pkgs';
        nedryland = nedryland.lib."${system}";
        nedryglot = nedryglot.lib."${system}";
        oxalica = oxalica.overlays.default;
      };
    in
    {
      lib = project // {
        override = projectFn;
      };
      packages = project.matrix // {
        default = project.components;
      };
      devShells = project.shells;
      apps = {
        bendini = {
          type = "app";
          program = "${project.matrix.bendini.rust}/bin/bendini";
        };
        lomax = {
          type = "app";
          program = "${project.matrix.lomax.rust}/bin/lomax";
        };
        avery = {
          type = "app";
          program = "${project.matrix.avery.rust}/bin/avery";
        };
        quinn = {
          type = "app";
          program = "${project.matrix.quinn.rust}/bin/quinn";
        };
      }
      // nedryland.apps.${system};
    });
}
