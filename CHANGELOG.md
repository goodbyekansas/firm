# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.2.1] - 2021-10-21
### Packages
- avery: 1.2.1
- bendini: 1.0.0
- firmRust: 1.0.0
- firmTypes-python: 1.0.0
- firmTypes-rust-withServices, firmTypes-rust-withoutServices: 1.0.0
- lomax: 1.0.1
- protocols-withServices-python, protocols-withServices-rust, protocols-withoutServices-python, protocols-withoutServices-rust: 1.0.0
- quinn: 1.0.0
- tonicMiddleware: 1.0.0
- windowsEvents: 0.1.0
- windowsInstaller: 0.1.1

### Fixed
- avery: Restrict permissions on named pipes. Before System, admins and user had RW. Everyone and anonymous users had R. Now only system user and user has RW.

## [1.2.0] - 2021-09-15
### Packages
- avery: 1.2.0
- bendini: 1.0.0
- firmRust: 1.0.0
- firmTypes-python: 1.0.0
- firmTypes-rust-withServices, firmTypes-rust-withoutServices: 1.0.0
- lomax: 1.0.1
- protocols-withServices-python, protocols-withServices-rust, protocols-withoutServices-python, protocols-withoutServices-rust: 1.0.0
- quinn: 1.0.0
- tonicMiddleware: 1.0.0
- windowsEvents: 0.1.0
- windowsInstaller: 0.1.1

### Fixed
- windowsInstaller: Make sure pending reboot deletions are correctly formatted
- lomax: Stop windows service on runtime errors correctly.
- avery: Stop windows service on runtime errors correctly.

### Added
- avery: Support for multiple hosted domains for authentication.

## [1.1.0] - 2021-09-09
### Added
- generated documentation for functions, can be customized in the project config under `docs.function` with the keys `css` and `jinja` pointing to such files.
- Extension for markdown to be used for changelogs and such.
- Shell to work with changelogs and github releases called `release`.
- windowsInstaller: Installs Avery as a user service.
- windowsInstaller: Installs Lomax as a system service.
- windowsInstaller: Installs Bendini.
- windowsInstaller: Adds the programs to PATH.
- windowsInstaller: Register Avery and Lomax to the event log.
- windowsInstaller: Uninstalls all this.
- windowsInstaller: Upgrade option to uninstall and then install and restarting services.
- avery: Avery can now be run as a user service on windows.
- avery: As a windows service avery writes to the event log.
- avery: Avery with runtimes now have windows targets.

### Packages
- avery: 1.1.0
- bendini: 1.0.0
- firmRust: 1.0.0
- firmTypes-python: 1.0.0
- firmTypes-rust-withServices, firmTypes-rust-withoutServices: 1.0.0
- lomax: 1.0.0
- protocols-withServices-python, protocols-withServices-rust, protocols-withoutServices-python, protocols-withoutServices-rust: 1.0.0
- quinn: 1.0.0
- tonicMiddleware: 1.0.0
- windowsEvents: 0.1.0
- windowsInstaller: 0.1.0

## [1.0.0] - 2021-07-03
### Packages
- avery: 1.0.0
- bendini: 1.0.0
- firmRust: 1.0.0
- firmTypes-python: 1.0.0
- firmTypes-rust-withServices, firmTypes-rust-withoutServices: 1.0.0
- protocols-withServices-python, protocols-withServices-rust, protocols-withoutServices-python, protocols-withoutServices-rust: 1.0.0
- lomax: 1.0.0
- quinn: 1.0.0
- tonicMiddleware: 1.0.0

