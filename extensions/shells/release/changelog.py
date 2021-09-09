""" Script to work with changelogs in firm"""
import argparse
import os
import pprint
import re
import sys
from functools import partial, reduce
from typing import Dict, List, Tuple, Union

import keepachangelog  # type: ignore # pylint: disable=import-error
import keepachangelog._versioning as versioning  # type: ignore # pylint: disable=import-error

# Turn on ANSI colors on windows
if os.name == "nt":
    os.system("color")


CHANGELOG = os.environ.get("FIRM_CHANGELOG", "file-not-found")


def padded_version(version: Union[Tuple[str, dict], str]) -> str:
    """ 3 digit pad semantic versions so they can be compared as strings safely"""
    if isinstance(version, str):
        sem_ver: Dict[str, Union[str, int]] = {}
        version_data, _, sem_ver["buildmetadata"] = version.partition("+")
        version_data, _, sem_ver["prerelease"] = version_data.partition("-")
        sem_ver["major"], sem_ver["minor"], sem_ver["patch"] = map(
            int, version_data.split(".")
        )
    else:
        sem_ver = version[1]["metadata"]["semantic_version"]
    sem_ver["prerelease"] = sem_ver["prerelease"] or ""
    sem_ver["buildmetadata"] = sem_ver["buildmetadata"] or ""
    return "{major:03}.{minor:03}.{patch:03}.{prerelease}.{buildmetadata}".format(
        **sem_ver
    )  # type: ignore


def find_latest(versions: dict) -> Tuple[str, dict]:
    """ Find the latest version (by semantic version) of a set of versions"""
    sorted_releases = sorted(
        filter(lambda x: x[0] != "unreleased", versions.items()), key=padded_version
    )

    if sorted_releases:
        return sorted_releases[-1]
    else:
        return ("", {})


def get_new_version(changelog: dict) -> str:
    """ Guess the next version number and ask for correction """
    latest = find_latest(changelog)
    new_version = versioning.guess_unreleased_version(
        changelog, latest[1].get("metadata", {}).get("semantic_version", { "major": 0, "minor": 0, "patch": 0, "prerelease": None, "buildmetadata": None})
    )
    print(f"Suggested version for these changes are: {new_version}.")
    return input("If this is incorrect input a new version here:") or new_version


def check_component(
    released: List[Tuple[str, str]], folder: str, component: str
) -> Tuple[str, str, dict]:
    """ Check if a component has a new version since the last firm release """
    _, version = next(filter(lambda p: p[0] == component, released), (None, "0.0.0"))
    changelog_file = os.path.join(folder, component)
    component_changelog = keepachangelog.to_dict(changelog_file, show_unreleased=True)
    latest = find_latest(component_changelog)
    if list(component_changelog.get("unreleased", {}).keys()) != ["metadata"]:
        print(f"\033[95m{component}\033[0m has unreleased changes since {version}:")
        unreleased = component_changelog.get("unreleased")
        unreleased.pop("metadata")
        print(re.sub(r"[\{\}\[\]]", "", pprint.pformat(unreleased, indent=2)))
        new_version = get_new_version(component_changelog)
        keepachangelog.release(changelog_file, new_version=new_version)
        print("")
        return (component, new_version, unreleased)

    if padded_version(latest) > padded_version(version):
        print(f"\033[95m{component}\033[0m has a new version {version} -> {latest[0]}:")
        meta = latest[1].pop("metadata")
        new_version = re.sub(r"[\{\}\[\]]", "", pprint.pformat(latest[1], indent=2))
        print(new_version)
        print("")
        return (component, meta["version"], latest[1])
    print(f"\033[95m{component}\033[0m has no changes")
    print("")
    return (component, version, {})


def combine_changelogs(accumulated: dict, current: Tuple[str, str, dict]) -> dict:
    """Combine changelog for a component with the main changelog"""
    if current[2]:
        for heading, changes in current[2].items():
            prepended = list(map(lambda c: f"{current[0]}: {c}", changes))
            accumulated["unreleased"].setdefault(heading, []).extend(prepended)
    accumulated["unreleased"].setdefault("packages", []).append(
        f"{current[0]}: {current[1]}"
    )
    accumulated["unreleased"]["packages"].sort()
    return accumulated


def write_changelog(changes: str) -> str:
    """Write the changelog and give it a version"""
    with open(CHANGELOG, "w") as changelog:
        changelog.write(changes)

    print("\033[95mFirm combined changes\033[0m")
    unreleased = keepachangelog.to_dict(CHANGELOG, show_unreleased=True).get(
        "unreleased", {}
    )
    unreleased.pop("metadata")
    print(re.sub(r"[\{\}\[\]]", "", pprint.pformat(unreleased, indent=2)))
    print("")
    new_version = get_new_version(
        keepachangelog.to_dict(CHANGELOG, show_unreleased=True)
    )
    keepachangelog.release(CHANGELOG, new_version=new_version)
    return new_version


def release(changelogs: str) -> None:
    """Here we go"""
    main_changelog = keepachangelog.to_dict(CHANGELOG, show_unreleased=True)
    last_packages = list(
        map(
            lambda p: p.partition(": ")[::2],
            find_latest(main_changelog)[1].get("packages", []),
        )
    )

    main_changelog["unreleased"]["packages"] = []
    check_component_since = partial(check_component, last_packages, changelogs)

    changes = keepachangelog.from_dict(
        reduce(
            combine_changelogs,
            map(check_component_since, os.listdir(changelogs)),
            main_changelog,
        )
    )
    released = write_changelog(changes)
    print(f"Updated changelog to release version {released}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("command")
    parser.add_argument("--changelogs")
    args = parser.parse_args()
    if args.command == "release":
        release(args.changelogs)
    elif args.command == "version":
        print(find_latest(keepachangelog.to_dict(CHANGELOG))[0])
    elif args.command == "description":
        latest_version = find_latest(keepachangelog.to_raw_dict(CHANGELOG))[1]
        print(latest_version.get("raw"))
    else:
        print(f'Unrecognized command "{args.command}"')
        sys.exit(1)
