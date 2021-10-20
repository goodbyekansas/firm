{ base, types, stdenv, lib, darwin, postgresql, coreutils, pkg-config, glibcLocales, openssl }:
base.languages.rust.mkService {
  name = "quinn";
  src = ./.;
  buildInputs = [ types.package openssl ]
    ++ lib.optional stdenv.hostPlatform.isDarwin darwin.apple_sdk.frameworks.Security;

  nativeBuildInputs = [ postgresql coreutils pkg-config ];

  extraChecks = ''
    source scripts/postgres.bash
    echo "running postgres tests..."
    postgres_tests
  '';

  LOCALE_ARCHIVE = if stdenv.isLinux then "${glibcLocales}/lib/locale/locale-archive" else "";
  shellHook = ''
    source scripts/postgres.bash
  '';
}
