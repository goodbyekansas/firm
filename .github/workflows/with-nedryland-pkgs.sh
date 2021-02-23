#! /bin/sh
NIX_PATH=nixpkgs=$(nix eval -f project.nix --raw nedryland.nixpkgsPath) exec "$@"
