{ base, protocols }:
base.languages.rust.nativeTools.mkLibrary {
  name = "libauth";
  src = ./.;
  buildInputs = [ protocols.withoutServices.rust ];
}
