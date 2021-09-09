{ base, mkShell, linkFarm, python38, lib, components, fetchFromGitHub, github-release }:
let
  components' = components { inherit (base) callFile callFunction; };
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
              (n: v: v != null)
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
          (changelog: component: { name = component; path = changelog; })
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
  shellHook = ''
    updateChangelogs() {
      if [ "$1" == "-h" ] || [ "$1" == "--help" ]; then
        echo "This tool updates the changelogs for: $(ls -m $allChangelogs | head)."
        echo
        echo "It will prompt you for adding version number to unreleased sections (with a suggestion)."
        echo "Then it will update the main changelog in $FIRM_CHANGELOG and add the changes for the components."
        echo "For example, if avery has:"
        echo -e "\033[1;94m  ### Added"
        echo "  -shiny new feautre"
        echo -e "  -other cool thing\033[0m"
        echo "Then the main changelog will get"
        echo -e "\033[1;94m  ### Added"
        echo "  -avery: shiny new feature"
        echo -e "  -avery: other cool thing\033[0m"
        echo "The main changelog will also get a ###Packages header with the version of each component."
      else
        python ${./release/changelog.py} release --changelogs ${allChangelogs}
      fi
    }

    makeRelease() {
        (
          set -e
          git checkout main
          git pull
          version=$(python ${./release/changelog.py} version)
          description=$(python ${./release/changelog.py} description)
          old_tags=$(git tag)
          if [[ $old_tags =~ "$version" ]]; then
            echo "$version is already tagged"
          else
            git tag -a "$version" -m "🔖 Firm $version"
            git push origin "$version"
          fi
          if [ -z "$GITHUB_TOKEN" ]; then
            if [ -n "$1" ]; then
              GITHUB_TOKEN=$(cat $1)
            else
              echo "No access token found and GITHUB_TOKEN was not set, can not make a github release remotely!"
              exit 1
            fi
          fi
          github-release release --tag "$version" --description "$description"
        )
    }

    export GITHUB_USER="goodbyekansas"
    export GITHUB_REPO="firm"
    FIRM_CHANGELOG="$(git rev-parse --show-toplevel)/CHANGELOG.md"
    export FIRM_CHANGELOG

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
