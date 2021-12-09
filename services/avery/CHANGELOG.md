# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed
- Startup of Avery is now side-effect free. This means that no keys are generated and no
  login will be required.
- Login is no longer automatic. Any request that would have required a login will now
  return a gRPC `unauthenticated`. An interactive client can then choose to call `login`
  to start an interactive login process. This process is carried out with the help of a
  stream of login commands which instructs the client which actions to take during the
  login process.

### Added
- Implemented endpoint for GetIdentity
- Implemented ListVersions endpoint
- Added publisher email filter for List and ListVersion

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
