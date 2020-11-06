{ base, pkgs, pythonVersion ? pkgs.python3 }:

base.languages.python.mkUtility {
  inherit pythonVersion;
  name = "firm-protocols";
  version = "0.1.0";
  src = ../.;
  nativeBuildInputs = (pythonPkgs: [ pythonPkgs.grpcio-tools pythonPkgs.mypy-protobuf ]);
  propagatedBuildInputs = (pythonPkgs: [ pythonPkgs.grpcio ]);
  doStandardTests = false;
  preBuild = ''
    mkdir ./firm_protocols
    cp python/setup.py .

    python -m grpc_tools.protoc \
        -I . \
        --python_out=./firm_protocols \
        --grpc_python_out=./firm_protocols \
        --mypy_out=./firm_protocols \
        **/*.proto

    # protoc does not add __init__.py files, so let's do so
    find ./firm_protocols -type d -exec touch {}/__init__.py \;

    find ./firm_protocols -type d -exec touch {}/py.typed \;

    shopt -s globstar
    shopt -s extglob
    shopt -s nullglob

    for pyfile in ./firm_protocols/**/*_grpc.py; do
      echo "# type: ignore" > ''${pyfile}i
    done

    # correct the imports since that is apparently impossible to do correctly
    sed -i -E 's/^from (\S* import .*_pb2)/from firm_protocols.\1/ ' firm_protocols/**/*.py
    sed -i -E 's/^from (\S* import .*_pb2)/from firm_protocols.\1/ ' firm_protocols/**/*.pyi
    sed -i -E 's/^from (\S*.*_pb2)/from firm_protocols.\1/ ' firm_protocols/**/*.pyi
  '';
}
