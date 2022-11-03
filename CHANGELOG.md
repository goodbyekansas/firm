# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed versions
- Update Nedryland to 7.0.0

## [4.1.0] - 2022-04-29

### Changed versions
- Update Nedryland to 6.0.0

## [4.0.0] - 2022-02-14

### Changed versions
- Update Nedryland to 4.0.0

### Packages
- avery: 2.0.2
- bendini: 2.0.0
- firmRust: 1.0.0
- firmTypes-python: 1.0.0
- firmTypes-rust: 1.0.0
- firmWindowsInstaller: 0.1.2
- lomax: 2.1.0
- protocols: 2.0.0
- quinn: 2.0.0
- tonicMiddleware: 1.0.0
- windowsInstall: 0.1.0

### Added
- lomax: Support for expansion of `{hostname}` to the real hostname in the
  `certificate_alt_names` config setting.

## [3.0.0] - 2022-01-21
### Packages
- avery: 2.0.2
- bendini: 2.0.0
- firmRust: 1.0.0
- firmTypes-python: 1.0.0
- firmTypes-rust-withServices, firmTypes-rust-withoutServices: 1.0.0
- firmWindowsInstaller: 0.1.2
- lomax: 2.0.0
- protocols-withServices-python, protocols-withServices-rust, protocols-withoutServices-python, protocols-withoutServices-rust: 2.0.0
- quinn: 2.0.0
- tonicMiddleware: 1.0.0
- windowsInstall: 0.1.0

### Fixed
- firmWindowsInstaller: Installer removing files it did not own during uninstall.
  The installer now only work on files it extracted itself. If you manually add
  extra files in for example the install directory it will ignore those during
  uninstall.
- firmWindowsInstaller: Issue where installer could not mark files for deletion.

### Added
- windowsInstall: Utility library for creating windows installers with data and services

## [2.0.3] - 2021-12-21
### Packages
- avery: 2.0.2
- bendini: 2.0.0
- firmRust: 1.0.0
- firmTypes-python: 1.0.0
- firmTypes-rust-withServices, firmTypes-rust-withoutServices: 1.0.0
- lomax: 2.0.0
- protocols-withServices-python, protocols-withServices-rust, protocols-withoutServices-python, protocols-withoutServices-rust: 2.0.0
- quinn: 2.0.0
- tonicMiddleware: 1.0.0
- windowsInstaller: 0.1.2

### Fixed
- windowsInstaller: Installer removing files it did not own during uninstall.
  The installer now only work on files it extracted itself. If you
  manually add extra files in for example the install directory
  it will ignore those during uninstall.
  windowsInstaller: Issue where installer could not mark files for deletion.

## [2.0.2] - 2021-12-17
### Packages
- avery: 2.0.2
- bendini: 2.0.0
- firmRust: 1.0.0
- firmTypes-python: 1.0.0
- firmTypes-rust-withServices, firmTypes-rust-withoutServices: 1.0.0
- lomax: 2.0.0
- protocols-withServices-python, protocols-withServices-rust, protocols-withoutServices-python, protocols-withoutServices-rust: 2.0.0
- quinn: 2.0.0
- tonicMiddleware: 1.0.0
- windowsInstaller: 0.1.1

### Fixed
- avery: Use `ListVersions` instead of `List` when executing functions. This makes sure
- that we only get one match.

## [2.0.1] - 2021-12-17
### Added
- Release shell reports success.

### Fixed
- The release shell's makeRelease now uses the provided github token correctly.
- Avery, Lomax, Quinn and Bendini having wrong versions in Cargo.toml.
- Fixed 2.0.0 changelog.
- avery: The generated keys used a hash of the DER format but when the key is saved and
- later read from disk it uses the PEM format, causing a hash mismatch. This changes the
- hash to avery: always use the PEM variant.

### Packages
- avery: 2.0.1
- bendini: 2.0.0
- firmRust: 1.0.0
- firmTypes-python: 1.0.0
- firmTypes-rust-withServices, firmTypes-rust-withoutServices: 1.0.0
- lomax: 2.0.0
- protocols-withServices-python, protocols-withServices-rust, protocols-withoutServices-python, protocols-withoutServices-rust: 2.0.0
- quinn: 2.0.0
- tonicMiddleware: 1.0.0
- windowsInstaller: 0.1.1

## [2.0.0] - 2021-12-16
### Packages
- avery: 2.0.0
- bendini: 2.0.0
- firmRust: 1.0.0
- firmTypes-python: 1.0.0
- firmTypes-rust-withServices, firmTypes-rust-withoutServices: 1.0.0
- lomax: 2.0.0
- protocols-withServices-python, protocols-withServices-rust, protocols-withoutServices-python, protocols-withoutServices-rust: 2.0.0
- quinn: 2.0.0
- tonicMiddleware: 1.0.0
- windowsInstaller: 0.1.1

### Added
- bendini: Support for interactive login. This handles `unauthenticated` errors from Avery by
- initiating an interactive login and handling server side commands. After a successful
- bendini: login, the original request is retried.
- bendini: Support for formatting options. Bendini now supports three format options for `list` and
- `list-versions`: `short`, `long` and `json`. `short` shows a condensed version of the
- list and `long` provides more details on each function. `json` will output the function
- list in JSON format (pretty-printed if stdout is a TTY). This can be useful for piping
- to other tools. Note that this should only be used for simple scripting purposes. For
- anything that does not fall into that category, use the actual gRPC API.
- bendini: `register` command takes publisher name and email if not provided they will be
- retrieved from the auth service.
- lomax: Removal of cancelled auth requests
- quinn: Quinn stores publisher, with name and email
- quinn: Implementation of ListVersions endpoint
- quinn: Added publisher email filter for List and ListVersion
- protocols: ListVersions endpoint for registry.
- protocols: publisher_email field for Filters in registry listings.
- protocols: Publisher field to FunctionData, Function, AttachmentData and Attachment.
- protocols: GetIdentity endpoint to auth.
- protocols: `Login` that performs an interactive login. This is done with a stream of
- protocols: `InteractiveLoginCommand` that the client follows.
- protocols: `WaitForRemoteAccessRequest` endpoint that can be used to wait for approval of a remote
- access request. This should be used together with gRPC timeouts.
- protocols: `CancelRemoteAccessRequest` endpoint to remove a pending remote access request.
- avery: Config can override JWT claims on private key files
- avery: Implemented endpoint for GetIdentity
- avery: Implemented ListVersions endpoint
- avery: Added publisher email filter for List and ListVersion
- avery: `WaitForRemoteAccessRequest` endpoint that can be used to wait for approval of a remote
- access request. This should be used together with gRPC timeouts.

### Changed
- quinn: Publisher table added, without migrating existing data
- protocols: Name filter is now just a string instead of a type
- avery: Public keys are now uploaded together with a key id making it possible to have multiple keys per users.
- avery: Startup of Avery is now side-effect free. This means that no keys are generated and no
- login will be required.
- avery: Login is no longer automatic. Any request that would have required a login will now
- return a gRPC `unauthenticated`. An interactive client can then choose to call `login`
- to start an interactive login process. This process is carried out with the help of a
- stream of login commands which instructs the client which actions to take during the
- login process.

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

