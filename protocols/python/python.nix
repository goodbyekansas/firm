{ base, pkgs, pythonVersion ? pkgs.python3 }:

base.languages.python.mkUtility {
  inherit pythonVersion;
  name = "gbk-protocols";
  version = "0.1.0";
  src = ../.;
  nativeBuildInputs = (pythonPkgs: [ pythonPkgs.grpcio-tools ]);
  doStandardTests = false;
  preBuild = ''
    mkdir ./gbk_protocols
    cp python/setup.py .

    python -m grpc_tools.protoc \
        -I . \
        --python_out=./gbk_protocols \
        --grpc_python_out=./gbk_protocols \
        **/*.proto

    # protoc does not add __init__.py files, so let's do so
    find ./gbk_protocols -type d -exec touch {}/__init__.py \;

    # correct the imports since that is apparently impossible to do correctly
    sed -i -E 's/^from (\S* import .*_pb2)/from gbk_protocols.\1/ ' gbk_protocols/**/*.py
  '';
}
