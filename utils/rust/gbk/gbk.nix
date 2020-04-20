{ base, pkgs }: base.languages.rust.mkUtility {
  name = "gbk";
  src = ./.;
  hasTests = false;
  defaultTarget = "wasm32-wasi";
  targets = [ "wasm32-wasi" ];
  useNightly = "2020-04-20";
}
