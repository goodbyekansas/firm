{ base, turboISLC }:
base.languages.rust.mkLibrary {
  name = "turbo-isl";
  src = ./.;

  propagatedNativeBuildInputs = [ turboISLC.package ];

  # For development use the component source directly
  shellCommands = {
    turbo = ''
      PYTHONPATH=$(realpath ..):$PYTHONPATH python -m turbo_islc.main "$@"
    '';
  };
}
