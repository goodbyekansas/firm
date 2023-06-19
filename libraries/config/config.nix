{ base, protocols }:
base.languages.rust.nativeTools.mkLibrary {
  name = "libconfig";
  src = ./.;
  buildInputs = [ protocols.withoutServices.rust ];
}
