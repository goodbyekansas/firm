# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed
- Name filter is now just a string instead of a type

### Added
- ListVersions endpoint for registry.
- publisher_email field for Filters in registry listings.
- Publisher field to FunctionData, Function, AttachmentData and Attachment.
- GetIdentity endpoint to auth.
- `Login` that performs an interactive login. This is done with a stream of
  `InteractiveLoginCommand` that the client follows.
- `WaitForRemoteAccessRequest` endpoint that can be used to wait for approval of a remote
  access request. This should be used together with gRPC timeouts.
- `CancelRemoteAccessReqest` endpoint to remove a pending remote access request.

## [1.0.0] - 2021-07-03

### Added
- auth
- function definition
- function registry
- function execution
