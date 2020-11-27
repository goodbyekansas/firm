{ pkgs, base, types }:
with pkgs;
base.languages.rust.mkService {
  name = "quinn";
  src = ./.;
  buildInputs = [ types.package pkgs.openssl ]
    ++ pkgs.stdenv.lib.optional pkgs.stdenv.hostPlatform.isDarwin pkgs.darwin.apple_sdk.frameworks.Security;

  nativeBuildInputs = [ pkgs.postgresql pkgs.coreutils pkgs.pkg-config ];

  extraChecks = ''
    source scripts/postgres.bash
    echo "running postgres tests..."
    postgres_tests
  '';

  LOCALE_ARCHIVE = if pkgs.stdenv.isLinux then "${pkgs.glibcLocales}/lib/locale/locale-archive" else "";
  shellHook = ''
    source scripts/postgres.bash
  '';
}
