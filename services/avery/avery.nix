{ stdenv
, base
, types
, tonicMiddleware
, targets ? [ ]
, defaultTarget ? ""
, darwin ? null
, xcbuild ? null
, pkgsCross ? null
, pkg-config
}:
base.languages.rust.mkService {
  inherit stdenv targets defaultTarget;
  name = "avery";
  src = ./.;
  srcExclude = [
    (path: type: (type == "directory" && baseNameOf path == "runtimes"))
    (path: type: (type == "regular" && baseNameOf path == "avery-with-runtimes.nix"))
  ];

  buildInputs = [ types.package tonicMiddleware.package ]
    ++ stdenv.lib.optional stdenv.hostPlatform.isWindows pkgsCross.mingwW64.windows.pthreads;
  nativeBuildInputs = [ pkg-config ] ++ stdenv.lib.optional stdenv.hostPlatform.isDarwin xcbuild;
}
