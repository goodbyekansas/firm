{ base, protocols }:
base.languages.rust.mkLibrary {
  name = "tonic-middleware";
  src = ./.;
  propagatedBuildInputs = [ protocols.package ];
}
