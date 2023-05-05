{ base, protocols, pkg-config, openssl }:
base.languages.rust.nativeTools.mkLibrary {
  name = "libfunction";
  src = ./.;
  buildInputs = [ protocols.withoutServices.rust openssl ];
  nativeBuildInputs = [ pkg-config ];
}
