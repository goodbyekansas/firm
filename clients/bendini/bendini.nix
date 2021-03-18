{ pkgs, base, types, tonicMiddleware, stdenv, targets ? [ ], defaultTarget ? "", pkgsCross ? null }:
base.languages.rust.mkClient {
  inherit stdenv targets defaultTarget;
  name = "bendini";
  src = ./.;
  buildInputs = [ types.package tonicMiddleware.package ]
    ++ stdenv.lib.optional stdenv.hostPlatform.isWindows pkgsCross.mingwW64.windows.pthreads;
  nativeBuildInputs = [ pkgs.pkg-config pkgs.openssl ];
}
