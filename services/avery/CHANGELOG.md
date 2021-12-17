# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed
- Use `ListVersions` instead of `List` when executing functions. This makes sure that we
  only get one match.

## [2.0.1] - 2021-12-17

### Fixed
- The generated keys used a hash of the DER format but when the key is saved and later
  read from disk it uses the PEM format, causing a hash mismatch. This changes the hash to
  always use the PEM variant.

## [2.0.0] - 2021-12-16

### Changed
- Public keys are now uploaded together with a key id making it possible to have multiple keys per users.
- Startup of Avery is now side-effect free. This means that no keys are generated and no
  login will be required.
- Login is no longer automatic. Any request that would have required a login will now
  return a gRPC `unauthenticated`. An interactive client can then choose to call `login`
  to start an interactive login process. This process is carried out with the help of a
  stream of login commands which instructs the client which actions to take during the
  login process.

### Added
- Config can override JWT claims on private key files
- Implemented endpoint for GetIdentity
- Implemented ListVersions endpoint
- Added publisher email filter for List and ListVersion
- `WaitForRemoteAccessRequest` endpoint that can be used to wait for approval of a remote
  access request. This should be used together with gRPC timeouts.

## [1.2.1] - 2021-10-21

### Fixed
- Restrict permissions on named pipes. Before System, admins and user had RW. Everyone and anonymous users had R. Now only system user and user has RW.

## [1.2.0] - 2021-09-15

### Fixed
- Stop windows service on runtime errors correctly.

### Added
- Support for multiple hosted domains for authentication.

## [1.1.0] - 2021-09-09

### Added
- Avery can now be run as a user service on windows.
- As a windows service avery writes to the event log.
- Avery with runtimes now have windows targets.

## [1.0.0] - 2021-07-03

### Added
- Authentication service.
- Registry service.
- Execution service.
- Wasi runtime.
- Python runtime.
