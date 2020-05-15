# Firm

![Checks](https://github.com/goodbyekansas/firm/workflows/Checks/badge.svg?branch=master)

The Firm is a collection of [Nedryland](https://github.com/goodbyekansas/nedryland) components that
adds the notion of a "function".

# Developer Setup

Firm is a [Nedryland](https://github.com/goodbyekansas/nedryland) project.

First, install [nix](https://nixos.org/nix/).

A development shell for any component can then be obtained by running

```sh
$ nix-shell -A <component> # component can be for example avery
```

Names of all components can be found in `project.nix` in the repo root.

# The Function

A function can be described as a self-contained unit that takes inputs and produces outputs.
However, it is not a function like in most programming languages in that it takes a set of inputs
and then execute until completion and then produces a single return value. Instead, a function in
this sense has multiple asynchronous inputs and multiple asynchronous outputs. A better way to think
about a function might be as a node in an asynchronous node graph.

# Execution of functions

A function is executed in something called an "execution environment". The function itself specifies
which execution environment it needs and Avery then tries to find a matching one. An execution
environment is simply something that accepts the function code (does not have to be actual code)
along with its' inputs and executes it. Note that a function itself can also be an execution
environment for other functions. Examples of execution environments could be WASI (for running in
the [Web Assembly System Interface](https://wasi.dev), Python (for running python code on the host),
etc.

# Components in the repo

## Avery

Avery is the heart of the functional pipeline. It is responsible for housing the base execution
environments that need to run on a host OS, downloading function code and starting the execution.
Avery also has a local development registry used for quick iteration when working on functions.

## Bendini

Bendini is a CLI interface to Avery. It has functionality for listing and executing functions.

## Lomax

Lomax is a CLI interface to the registry API for functions. It therefore has functionality to list
and register functions.

## Wasi Function Utils

A Rust library that houses utility functionality for Rust functions targeting the WASI execution
environment. Contains helpers to get inputs, set outputs and set errors etc.

## Nedryland Extension

`nedryland/function.nix` contains a Nedryland extension that adds the notion of a "function"
microservice component. This component is added to the base set of components that Nedryland already
supports.
