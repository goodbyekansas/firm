#! /bin/sh
NIX_PATH=nixpkgs=$(nix eval -f project.nix --raw nixpkgsPath) exec "$@"
