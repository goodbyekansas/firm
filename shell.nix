{ pkgs, components, extraShells ? { } }:
let
  getAllPackages = components:
    let
      comp = if (builtins.isFunction components) then (components { }) else components;
    in
    [ ]
    ++ (
      if builtins.hasAttr "package" comp then
        [ comp.package ]
      else
        builtins.map (c: getAllPackages c) (builtins.attrValues comp)
    );

  all = pkgs.mkShell {
    buildInputs = (getAllPackages components);
  };
in
pkgs.lib.mapAttrsRecursiveCond
  (current: !(builtins.hasAttr "package" current && builtins.hasAttr "packageWithChecks" current))
  (
    attrName: component:
      (
        let
          comp = if builtins.isFunction component then (component { }) else component;
          pkg = comp.packageWithChecks;
          name = comp.name or (builtins.concatStringsSep "." attrName);
          shellPkg = pkg.drvAttrs // {
            name = "${pkg.name}-shell";
            nativeBuildInputs = (pkg.shellInputs or [ ]) ++ (pkg.nativeBuildInputs or [ ]);
            componentDir = builtins.toString comp.path;
            shellHook = ''
              componentDir="$componentDir"
              if [ -f "$componentDir" ]; then
                componentDir=$(dirname "$componentDir")
              fi

              echo ⛑ Changing dir to \"$componentDir\"
              cd "$componentDir"
              echo 🐚 Running shell hook for \"${name}\"
              ${pkg.shellHook or ""}
              echo 🥂 You are now in a shell for working on \"${name}\"
            '';
          };
        in
        pkgs.mkShell shellPkg
      )
  )
  components // extraShells // {
  inherit all;
}
