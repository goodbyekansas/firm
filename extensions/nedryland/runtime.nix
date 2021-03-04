{ base, pkgs, bendini, avery }:
let
  mkRuntime = attrs@{ name, runtimeName ? name, src, examples ? { }, fileSystemImage ? null, ... }:
    let
      env = (builtins.mapAttrs
        (n: v: {
          function = v.deployment.function;
          name = v.package.name;
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
      runner = pkgs.runCommand "runtime-runner-${name}.bash"
        {
          inherit fileSystemImage;
          name = runtimeName;
          preferLocalBuild = true;
          allowSubstitutes = false;
          functions = envVars;
        }
        ''
          substituteAll ${./runtime-runner.bash} $out
          chmod +x $out
        '';
      mkPackage = base.languages.rust.mkPackage.override { stdenv = pkgs.pkgsCross.wasi32.clang11Stdenv; };
      curatedAttrs = builtins.removeAttrs attrs [ "name" "src" "examples" "fileSystemImage" "runtimeName" ];
      extension = if fileSystemImage == null then ".wasm" else ".tar.gz";
    in
    base.mkComponent {
      inherit name runtimeName examples;
      package = mkPackage (curatedAttrs // {
        inherit name src;
        targets = [ "wasm32-wasi" ];
        defaultTarget = "wasm32-wasi";
        nativeBuildInputs = [ pkgs.wasmtime ] ++ pkgs.lib.optional pkgs.stdenv.isDarwin pkgs.llvmPackages_11.llvm ++ curatedAttrs.nativeBuildInputs or [ ];
        shellInputs = [ pkgs.wabt pkgs.coreutils bendini.package avery.package ] ++ curatedAttrs.shellInputs or [ ];

        CARGO_TARGET_WASM32_WASI_RUNNER = runner;
        useNightly = curatedAttrs.useNightly or "2021-03-04";
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
      });
    };
in
base.extend.mkExtension {
  componentTypes = base.extend.mkComponentType {
    name = "runtime";
    createFunction = mkRuntime;
  };
}
