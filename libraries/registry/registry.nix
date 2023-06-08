{ base, protocols }:
base.languages.rust.nativeTools.mkLibrary {
  name = "libregistry";
  src = ./.;
  buildInputs = [ protocols.withoutServices.rust ];
}
