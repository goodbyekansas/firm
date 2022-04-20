{ base }:
(base.languages.python.mkClient {
  name = "turbo_islc";
  version = "0.1.0";
  src = ./.;
  propagatedBuildInputs = (p: [ p.jinja2 p.pyparsing ]);

  shellCommands = {
    run = ''
      python -m turbo_islc.main "$@"
    '';
  };
}).overrideAttrs(_:
  {
    rust = base.callFile ./rust/rust.nix {};
  }
)
