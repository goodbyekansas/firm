{ base, protocols }:
base.languages.rust.latestStable.mkLibrary {
  name = "libauth";
  src = ./.;
  buildInputs = [ protocols.withoutServices.rust ];
}
