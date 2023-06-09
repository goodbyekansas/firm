{ base, protocols, libraries }:
base.languages.rust.nativeTools.mkLibrary {
  name = "libregistry";
  src = ./.;
  buildInputs = [ protocols.withoutServices.rust libraries.function ];
}
