# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Support for formatting options. Bendini now supports three format options for `list` and
  `list-versions`: `short`, `long` and `json`. `short` shows a condensed version of the
  list and `long` provides more details on each function. `json` will output the function
  list in JSON format (pretty-printed if stdout is a TTY). This can be useful for piping
  to other tools. Note that this should only be used for simple scripting purposes. For
  anything that does not fall into that category, use the actual gRPC API.

- `register` command takes publisher name and email if not provided they will be
  retrieved from the auth service.

## [1.0.0] - 2021-07-03

### Added
- Command for listing functions.
- Command for executing functions.
- Command for listing runtimes.
- Command for list auth requests.
- Command for approving auth requests.
