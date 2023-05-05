{ base, protocols, libraries }:
base.languages.rust.nativeTools.mkLibrary {
  name = "libruntime";
  src = ./.;
  buildInputs = [ protocols.withoutServices.rust libraries.function ];
}
