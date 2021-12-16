{ base, mkShell, linkFarm, python38, lib, components, github-release }:
let
  components' = components { inherit (base) callFile; };
  allChangelogs =
    let
      toList = set: if set ? changelog then set else (builtins.map toList (builtins.attrValues set));
      # This is a bit messy because:
      # 1. We have nested components
      # 2. We have "internal" components, without changelogs
      # 3. We have components that share a changelog
      allComponentChangelogs = (lib.flatten
        (builtins.map
          toList
          (builtins.attrValues
            (lib.filterAttrsRecursive
              (_: v: v != null)
              (lib.mapAttrsRecursiveCond
                (attr: !(attr ? isNedrylandComponent))
                (
                  names: comp:
                    if comp ? path && (builtins.readDir (builtins.dirOf comp.path)) ? "CHANGELOG.md" then
                      {
                        component = builtins.concatStringsSep "-" names;
                        changelog = builtins.toString ((builtins.dirOf comp.path) + /CHANGELOG.md);
                      }
                    else null
                )
                components')))));

      uniqueChangelogs =
        lib.mapAttrsToList
          (path: name: { inherit name path; })
          (builtins.foldl'
            (acc: cur:
              if acc ? "${cur.changelog}" then
                acc // { "${cur.changelog}" = "${acc."${cur.changelog}"}, ${cur.component}"; }
              else
                acc // { "${cur.changelog}" = cur.component; }
            )
            { }
            allComponentChangelogs);
    in
    linkFarm
      "firm-changelogs"
      uniqueChangelogs;
in
mkShell {
  buildInputs = [ python38 python38.pkgs.keepachangelog github-release ];
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
    echo -e "\033[1;96mmakeRelease\033[0m <access token path>"
    echo "  Creates a tag at the current commit on main, pushes it and makes a github release."
    echo '  Requires a personal access token to be entered or $GITHUB_TOKEN to be set'
    echo
    echo -e "\033[1;96mgithub-release\033[0m"
    echo "  For manually working with github releases from the command line (this is used internally by makeRelease)."
    echo "  GITHUB_USER and GITHUB_REPO has been set so --user and --repo arguments are not necessary."
    echo "  see https://github.com/github-release/github-release/tree/$(github-release --version | sed 's/^.* \(.*\)$/\1/') for more info"

  '';
}
