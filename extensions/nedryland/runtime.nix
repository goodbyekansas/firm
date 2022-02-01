{ base, pkgs, bendini, avery }:
let
  mkRuntime = attrs@{ name, runtimeName ? name, src, examples ? { }, fileSystemImage ? null, ... }:
    let
      env = (builtins.mapAttrs
        (_: v: {
          function = v.deployment.function;
          name = v.package.pname;
        })
        examples);

      envVars = pkgs.writeTextFile {
        name = "runtime-runner-env";
        text = builtins.foldl'
          (acc: curr: ''
            ${acc}
            declare -x ${curr}="${pkgs.lib.strings.escape [ "$" "\"" ] (builtins.toString (builtins.getAttr curr env).function)}"
            declare -x ${curr}_name="${(builtins.getAttr curr env).name}"
          '')
          ""
          (builtins.attrNames env);
      };

      runner = pkgs.runCommand "runtime-runner-${name}"
        {
          inherit fileSystemImage;
          name = runtimeName;
          preferLocalBuild = true;
          allowSubstitutes = false;
          functions = envVars;
        }
        ''
          mkdir -p $out/bin
          substituteAll ${./runtime/runtime-runner.bash} $out/bin/runtime-runner
          chmod +x $out/bin/runtime-runner
        '';

      curatedAttrs = builtins.removeAttrs attrs [ "name" "src" "examples" "fileSystemImage" "runtimeName" ];
      extension = if fileSystemImage == null then ".wasm" else ".tar.gz";
    in
    (base.languages.rust.mkComponent (curatedAttrs // {
      inherit name src;
      nedrylandType = "avery-runtime";
      defaultTarget = "wasi";

      nativeBuildInputs = [ pkgs.wasmtime ] ++ pkgs.lib.optional pkgs.stdenv.isDarwin pkgs.llvmPackages_12.llvm ++ curatedAttrs.nativeBuildInputs or [ ];
      shellInputs = [ pkgs.wabt pkgs.coreutils bendini avery runner ] ++ curatedAttrs.shellInputs or [ ];
      extraCargoConfig = attrs.extraCargoConfig or "";
      checkInputs = pkgs.lib.optional curatedAttrs.exposeRunnerInChecks or false runner ++
        curatedAttrs.checkInputs or [ ];

      shellHook = ''
        export CARGO_TARGET_WASM32_WASI_RUNNER=runtime-runner
        ${attrs.shellHook or ""}
      '';

      useNightly = curatedAttrs.useNightly or "2021-11-22";
      installPhase = ''
        mkdir -p $out/share/avery/runtimes
        cp target/wasm32-wasi/release/*.wasm $out/share/avery/runtimes/${runtimeName}.wasm
        ${curatedAttrs.installPhase or ""}
      '';

      postFixup = ''
        executableSha=$(sha256sum $out/share/avery/runtimes/${runtimeName}.wasm | cut -d ' ' -f 1)
        sha=$executableSha
        ${if fileSystemImage != null then ''
            ln -s ${fileSystemImage} fs

            # -h to resolve symlinks
            # also set mode because of https://github.com/alexcrichton/tar-rs/issues/242
            echo "📦 creating tar file for runtime filesystem image..."
            tar -chzf "$out/share/avery/runtimes/${runtimeName}.tar.gz" --mode='a+rwX' fs \
                -C $out/share/avery/runtimes/ --owner 0 --group 0 ${runtimeName}.wasm
            echo "🌅 Image created!"

            sha=$(sha256sum $out/share/avery/runtimes/${runtimeName}.tar.gz | cut -d ' ' -f 1)
            rm $out/share/avery/runtimes/${runtimeName}.wasm
          ''
          else ""}

        echo "
        [\"${runtimeName}${extension}\"]
        sha256=\"$sha\"
        executable_sha256=\"$executableSha\"" >"$out/share/avery/runtimes/.checksums.toml"
      '';
    })).overrideAttrs (_: {
      inherit runtimeName examples;
    });
in
base.extend.mkExtension {
  componentTypes = base.extend.mkComponentType {
    name = "runtime";
    createFunction = mkRuntime;
  };
}
