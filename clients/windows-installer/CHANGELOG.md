# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed
- Installer removing files it did not own during uninstall.
  The installer now only work on files it extracted itself. If you
  manually add extra files in for example the install directory
  it will ignore those during uninstall.

- Issue where installer could not mark files for deletion.

## [0.1.1] - 2021-09-15

### Fixed
- Make sure pending reboot deletions are correctly formatted

## [0.1.0] - 2021-09-09

### Added
- Installs Avery as a user service.
- Installs Lomax as a system service.
- Installs Bendini.
- Adds the programs to PATH.
- Register Avery and Lomax to the event log.
- Uninstalls all this.
- Upgrade option to uninstall and then install and restarting services.
