{ base, protocols }:
base.languages.rust.nativeTools.mkLibrary {
  name = "libfunction";
  src = ./.;
  buildInputs = [ protocols.withoutServices.rust ];
}
