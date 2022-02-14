{ base, mkShell, linkFarm, python38, lib, components, gh }:
let
  allChangelogs =
    let
      allComponentChangelogs = base.collectComponentsRecursive (base.mapComponentsRecursive
        (
          namePath: comp:
            lib.optionalAttrs (comp ? path && (builtins.readDir (builtins.dirOf comp.path)) ? "CHANGELOG.md")
              {
                isNedrylandComponent = true;
                inherit namePath;
                changelog = builtins.toString ((builtins.dirOf comp.path) + /CHANGELOG.md);
              }
        )
        components);

      longestCommonList =
        let
          longestCommonList' = curr: a: b:
            if a != [ ] && b != [ ] && builtins.head a == builtins.head b then
              longestCommonList'
                (curr ++ [ (builtins.head a) ])
                (builtins.tail a)
                (builtins.tail b)
            else
              curr;

        in
        longestCommonList' [ ];

      uniqueChangelogs =
        lib.mapAttrsToList
          (path: name: { inherit path; name = builtins.concatStringsSep "-" name; })
          (builtins.foldl'
            (acc: cur:
              acc //
              (if acc ? "${cur.changelog}" then
                { "${cur.changelog}" = longestCommonList acc."${cur.changelog}" cur.namePath; }
              else
                { "${cur.changelog}" = cur.namePath; })
            )
            { }
            allComponentChangelogs);
    in
    linkFarm
      "firm-changelogs"
      uniqueChangelogs;
in
mkShell {
  nativeBuildInputs = [ python38 python38.pkgs.keepachangelog gh ];
  inherit allChangelogs;
  CHANGELOG_SCRIPT = ./release/changelog.py;
  shellHook = ''
    source ${./release/shell-scripts.bash}
    echo -e "🚀 \033[1mWelcome to the release shell!\033[0m"
    echo "The following tools are available in this shell:"
    echo
    echo -e "\033[1;96mupdateChangelogs\033[0m"
    echo "  Updates and gathers all changelogs for all components and put them in the main CHANGELOG.md."
    echo "  Use updateChangelogs --help for more info."
    echo
    echo -e "\033[1;96mmakeRelease\033[0m"
    echo "  Creates a tag at the current commit on main, pushes it and makes a github release."
  '';
}
