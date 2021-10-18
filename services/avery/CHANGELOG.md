# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
