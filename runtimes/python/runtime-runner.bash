#! /usr/bin/env bash
original_flags=$-
set -e

trap_command="set $original_flags"
trap_handler() {
    echo "🧹 Cleaning up..."
    eval "$trap_command"
}
trap trap_handler EXIT

# if no example given, assume we want to run
# TODO: there might be a better way to do this...
if [ -z "$2" ]; then
    echo "no example name given, running WASI executable..."
    tmpdir=$(mktemp -d)
    wasmtime run --disable-cache --env=RUST_TEST_NOCAPTURE=1 --mapdir=.::$tmpdir "$@"
    exitCode=$?
    rm -rf $tmpdir
    exit $exitCode
fi

# source a definition of all the functions
# we have available
source @functions@

# argument "parsing"
executable=$(realpath "$1")
shift

example="$1"
if [ -z "${!example}" ]; then
    echo "Failed to find example \"${example}\""
    exit 1
fi
shift

# create a runtime directory containing an avery
# config and a directory of runtime(s)
avery_dir=$(mktemp -d avery-runtime-runner-XXXXXXXX --tmpdir)
trap_command+="; rm -rf $avery_dir"

runtime_dir="$avery_dir/runtimes"
mkdir -p "$runtime_dir"

fsimage="@fileSystemImage@"
if [ -n "${fsimage:-}" ]; then
    runtime="@name@.tar.gz"

    # TODO: Add a python script that does this instead
    # since the tarfile module in python can actually build
    # a tar that is not *exactly* the structure on disk
    # and this is silly
    tmp_tar_dir=$(mktemp -d --tmpdir runtime-archive.XXXXXXX)
    trap_command+="; rm -rf $tmp_tar_dir"

    ln -s "$fsimage" "$tmp_tar_dir/fs"

    # -h to resolve symlinks
    # also set mode because of https://github.com/alexcrichton/tar-rs/issues/242
    echo "📦 creating tar file for runtime filesystem image..."
    tar -chzf "$runtime_dir/$runtime" --mode='a+rwX' -C "$tmp_tar_dir" fs -C "$(dirname $executable)" "$(basename $executable)"
    echo "🌅 Image created!"
else
    runtime="$(basename executable)"
    cp "$executable" "$runtime_dir"
fi

# avery config file
echo "
[\"$runtime\"]
sha256=\"$(sha256sum $runtime_dir/$runtime | cut -d ' ' -f 1)\"
executable_sha256=\"$(sha256sum $executable | cut -d ' ' -f 1)\"" >"$runtime_dir/.checksums.toml"

# checksum file for the avery filesystem source
echo "
runtime_directories=[\"$runtime_dir\"]
[internal_registry]
version_suffix=\"\"" >"$avery_dir/avery.toml"

avery --config "$avery_dir/avery.toml" &
trap_command+="; kill -s SIGTERM %1 && wait %1"

echo "🏒 Running example ${example} at ${!example}"
${!example}/bin/deploy

namevar="${example}_name"
command bendini run "${!namevar}:*" --follow "$@"
