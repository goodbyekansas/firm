if builtins.getEnv "NEDRYLAND_PATH" != "" then
  (./. + "/${builtins.getEnv "NEDRYLAND_PATH"}")
else
  builtins.fetchGit {
    name = "nedryland";
    url = "git@github.com:goodbyekansas/nedryland.git";
    ref = "4505394a1d52de47483f8205c016aa99c5c6fa19";
  }
