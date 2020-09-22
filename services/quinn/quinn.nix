{ pkgs, base, protocols, protocolsTestHelpers }:
with pkgs;
base.languages.rust.mkService {
  name = "quinn";
  src = ./.;
  rustDependencies = [ protocols protocolsTestHelpers ];

  nativeBuildInputs = [ pkgs.postgresql pkgs.coreutils ];

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
