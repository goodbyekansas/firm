{ pkgs }:
let
  hostPython = pkgs.pkgsCross.wasi32.buildPackages.python38;
  buildWasiWheel =
    { name
    , version
    , src
    , setupPyArgs ? ""
    , dependencies ? [ ]
    }:
    pkgs.pkgsCross.wasi32.clang12Stdenv.mkDerivation rec {
      pname = name;
      inherit src version;

      nativeBuildInputs = [ hostPython hostPython.pkgs.setuptools hostPython.pkgs.wheel ];
      propagatedBuildInputs = dependencies;

      buildPhase = ''
        python setup.py ${setupPyArgs} bdist_wheel 
      '';

      installPhase = ''
        mkdir -p $out/firm

        echo $propagatedBuildInputs >>$out/firm/wasi-dependencies

        for pi in $propagatedBuildInputs; do
          if [ -f $pi/firm/wasi-dependencies/ ]; then
            echo -n " " >>$out/firm/wasi-dependencies
            cat $pi/firm/wasi-dependencies >>$out/firm/wasi-dependencies
          fi
        done

        mkdir -p $out/lib/wasi-wheels
        cp dist/*.whl $out/lib/wasi-wheels
      '';
    };
in
rec {
  idna = buildWasiWheel {
    name = "idna";
    version = "2.10";
    src = hostPython.pkgs.idna.src;
  };

  chardet = buildWasiWheel rec {
    name = "chardet";
    version = "4.0.0";
    src = hostPython.pkgs.fetchPypi {
      pname = name;
      inherit version;
      sha256 = "1ykr04qyhgpc0h5b7dhqw4g92b1xv7ki2ky910mhy4mlbnhm6vqd";
    };
  };

  urllib3 = buildWasiWheel rec {
    name = "urllib3";
    version = "1.26.3";

    src = hostPython.pkgs.fetchPypi {
      pname = name;
      inherit version;
      sha256 = "0wvyljq6qh8a0ahs5456bzaz88kzip6ha8189qrq69jasymfsgny";

    };
  };

  certifi = buildWasiWheel rec {
    name = "certifi";
    version = "2020.12.5";

    src = hostPython.pkgs.fetchPypi {
      pname = name;
      inherit version;
      sha256 = "177mdbw0livdjvp17sz6wsfrc32838m9y59v871gpgv2888raj8s";
    };
  };

  requests = buildWasiWheel rec {
    name = "requests";
    version = "2.25.1";
    dependencies = [ idna chardet urllib3 certifi ];
    src = hostPython.pkgs.fetchPypi {
      pname = name;
      inherit version;
      sha256 = "015qflyqsgsz09gnar69s6ga74ivq5kch69s4qxz3904m7a3v5r7";
    };
  };

  pyyaml = buildWasiWheel rec {
    name = "PyYAML";
    version = "5.4.1";
    dependencies = [ ];
    src = hostPython.pkgs.fetchPypi {
      pname = name;
      inherit version;
      sha256 = "0pm440pmpvgv5rbbnm8hk4qga5a292kvlm1bh3x2nwr8pb5p8xv0";
    };
  };
}
