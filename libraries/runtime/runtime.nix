{ base, protocols }:
base.languages.rust.nativeTools.mkLibrary {
  name = "libruntime";
  src = ./.;
  buildInputs = [ protocols.withoutServices.rust ];
}
