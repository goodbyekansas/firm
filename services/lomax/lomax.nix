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
  name = "lomax";
  src = ./.;

  buildInputs = [ types.package tonicMiddleware.package ]
    ++ stdenv.lib.optional stdenv.hostPlatform.isWindows pkgsCross.mingwW64.windows.pthreads;
  nativeBuildInputs = [ pkg-config ] ++ stdenv.lib.optional stdenv.hostPlatform.isDarwin xcbuild;
}