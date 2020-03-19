if builtins.getEnv "NEDRYLAND_PATH" != "" then
  (./. + "/${builtins.getEnv "NEDRYLAND_PATH"}")
else
  builtins.fetchGit {
    name = "nedryland";
    url = "git@github.com:goodbyekansas/nedryland.git";
    ref = "fcf187f0a0cbd9f85bd736c20e79e458b01f92a5";
  }

