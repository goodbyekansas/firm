#! /usr/bin/env bash

updateChangelogs() {
  if [ "$1" == "-h" ] || [ "$1" == "--help" ]; then
    echo "This tool updates the changelogs for: $(ls -m "${allChangelogs:-}")."
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
    python "$CHANGELOG_SCRIPT" release --changelogs "$allChangelogs"
    echo "Remember to update Cargo.toml with correct versions!"
  fi
}

# TODO: Automate to update Cargo.toml files with correct versions.

makeRelease() {
  (
    set -e
    git checkout main
    git pull
    version=$(python "$CHANGELOG_SCRIPT" version)
    description=$(python "$CHANGELOG_SCRIPT" description)
    old_tags=$(git tag)
    if [[ $old_tags =~ $version ]]; then
      echo "$version is already tagged"
    else
      git tag -a "$version" -m "🔖 Firm $version"
      git push origin "$version"
    fi
    if [ -z "$GITHUB_TOKEN" ]; then
      if [ -n "$1" ]; then
        GITHUB_TOKEN=$(cat "$1")
      else
        echo "No access token found and GITHUB_TOKEN was not set, can not make a github release remotely!"
        exit 1
      fi
    fi
    export GITHUB_TOKEN
    github-release release --tag "$version" --description "$description"
    echo "Release \"$version\" done! 📦"
  )
}

export GITHUB_USER="goodbyekansas"
export GITHUB_REPO="firm"
FIRM_CHANGELOG="$(git rev-parse --show-toplevel)/CHANGELOG.md"
export FIRM_CHANGELOG
