if builtins.getEnv "NEDRYLAND_PATH" != "" then
  (./. + "/${builtins.getEnv "NEDRYLAND_PATH"}")
else
  builtins.fetchGit {
    name = "nedryland";
    url = "git@github.com:goodbyekansas/nedryland.git";
  }

