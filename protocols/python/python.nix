{ base, pkgs, protoSources, pythonVersion ? pkgs.python3 }:

base.languages.python.mkUtility {
  inherit pythonVersion protoSources;
  name = "firm-protocols";
  version = "0.1.0";
  nativeBuildInputs = (pythonPkgs: [ pythonPkgs.grpcio-tools pythonPkgs.mypy-protobuf pythonPkgs.mypy ]);
  src = ./.;
  propagatedBuildInputs = (pythonPkgs: [ pythonPkgs.grpcio ]);
  doStandardTests = false;
  preBuild = ''
    mkdir ./firm_protocols

    python -m grpc_tools.protoc \
        -I $protoSources \
        --python_out=./firm_protocols \
        --grpc_python_out=./firm_protocols \
        --mypy_out=./firm_protocols \
        $protoSources/**/*.proto

    # protoc does not add __init__.py files, so let's do so
    find ./firm_protocols -type d -exec touch {}/__init__.py \;

    find ./firm_protocols -type d -exec touch {}/py.typed \;

    shopt -s globstar
    shopt -s extglob
    shopt -s nullglob

    for pyfile in ./firm_protocols/**/*_grpc.py; do
      stubgen $pyfile -o .

      # Correcting some mistakes made by stubgen.
      # Generate static methods without return types. We just replace that with any return type.
      sed -i -E 's/\):/\) -> Any:/' ''${pyfile}i
    done

    # correct the imports since that is apparently impossible to do correctly
    sed -i -E 's/^from (\S* import .*_pb2)/from firm_protocols.\1/ ' firm_protocols/**/*.py
    sed -i -E 's/^from (\S* import .*_pb2)/from firm_protocols.\1/ ' firm_protocols/**/*.pyi
    sed -i -E 's/^from (\S*.*_pb2)/from firm_protocols.\1/ ' firm_protocols/**/*.pyi
  '';
}
