let
  configuredGrid = import ./default.nix;
in
  import ((import ./nedryland.nix) + "/shell.nix") { components = configuredGrid.components; }
