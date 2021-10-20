pkgs: parseConfig: manifest:
let
  title = builtins.replaceStrings
    [ "-" ]
    [ " " ]
    (pkgs.lib.toUpper (builtins.substring 0 1 manifest.name) + builtins.substring 1 (builtins.stringLength manifest.name) manifest.name);
  required_inputs = (pkgs.lib.filterAttrs (_: v: v.required or false) manifest.inputs);
  optional_inputs = (pkgs.lib.filterAttrs (_: v: !v.required or false) manifest.inputs);
  manifestData = builtins.toFile
    "manifest-data.json"
    (builtins.toJSON {
      manifest = {
        inherit required_inputs optional_inputs title;
        inherit (manifest) name version inputs outputs;
        description = manifest.description or "";
        metadata = manifest.metadata or { };
        runtime = manifest.runtime.type;
        attachments = builtins.attrNames (manifest.attachments or { });
      };
    });
  jinjaTemplate = ./index.html.jinja2;
  overrides = pkgs.lib.filterAttrs
    (_: v: v != null)
    (parseConfig {
      key = "docs";
      structure = {
        function = {
          css = null;
          jinja = null;
        };
      };
    }).function;
in
pkgs.stdenv.mkDerivation {
  name = "${manifest.name}-generated-docs";
  buildInputs = [ pkgs.python3Packages.j2cli ];
  src = ./index.html.jinja2;
  builder = builtins.toFile "builder.sh" ''
    source $stdenv/setup
    mkdir -p $out
    ${if overrides ? jinja then "ln -s ${overrides.jinja} extension.html" else ""}
    ln -s ${jinjaTemplate} index.html
    j2 \
      --format json  \
      --customize ${./template_settings.py} \
      ${if overrides ? jinja then "extension.html" else "index.html"} ${manifestData} \
      -o $out/index.html
    cp ${if overrides ? css then "${overrides.css}" else "${./styles.css}"} $out/styles.css
  '';
}
