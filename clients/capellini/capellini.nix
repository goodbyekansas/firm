{ base, fuse, pkg-config }:
base.languages.rust.nightly.mkClient {
  name = "capellini";
  src = ./.;
  nativeBuildInputs = [ pkg-config ];
  buildInputs = [ fuse ];
}
