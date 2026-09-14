# Draft package expressions; see README.md for the validation limits.
{ pkgs ? import <nixpkgs> { } }:

let
  inherit (pkgs) lib;
  version = "0.3.3";
  src = pkgs.fetchurl {
    url = "https://github.com/robert-cronin/flere/releases/download/v${version}/flere-${version}-source.tar.gz";
    # SHA-256 of the compressed release archive, not an unpacked/NAR hash.
    hash = "sha256-Rhu+Hj+ogCfC6nGRw0rb1+zqrb9vCRonB2mxzcCtap4=";
  };

  coreDeps = pkgs.rustPlatform.importCargoLock { lockFile = ./locks/core-Cargo.lock; };
  companionDeps = pkgs.rustPlatform.importCargoLock { lockFile = ./locks/companion-Cargo.lock; };
  # The core integration suite invokes a real offline sibling Cargo build.
  # Merge checksum-derived vendor directories, never synthesize a new lockfile.
  combinedDeps = lockFile: pkgs.runCommand "cargo-vendor-dir" {
    core_deps = coreDeps;
    companion_deps = companionDeps;
    selected_lock = lockFile;
  } (builtins.readFile ./merge-vendors.sh);
  fixtureTools = pkgs.writeText "flere-test-tools.json" (builtins.toJSON {
    "/bin/sh" = "${pkgs.bash}/bin/sh";
    "/bin/bash" = "${pkgs.bashInteractive}/bin/bash";
    "/bin/zsh" = "${pkgs.zsh}/bin/zsh";
    "/bin/sleep" = "${pkgs.coreutils}/bin/sleep";
    "/usr/bin/env" = "${pkgs.coreutils}/bin/env";
    "/usr/bin/python3" = "${pkgs.python3}/bin/python3";
    "/usr/bin/vim" = "${pkgs.vim}/bin/vim";
    "/usr/bin/git" = "${pkgs.git}/bin/git";
    "/usr/bin/false" = "${pkgs.coreutils}/bin/false";
    "/usr/bin/true" = "${pkgs.coreutils}/bin/true";
    "/usr/bin/yes" = "${pkgs.coreutils}/bin/yes";
    "/usr/bin/printf" = "${pkgs.coreutils}/bin/printf";
    "/usr/bin/stat" = "${pkgs.coreutils}/bin/stat";
    "/usr/bin/sha256sum" = "${pkgs.coreutils}/bin/sha256sum";
    "/usr/bin:/bin" = lib.makeBinPath [ pkgs.coreutils pkgs.bash pkgs.gnused pkgs.gnugrep ];
  });

  mkPackage =
    {
      pname,
      description,
      cargoRoot,
      lockFile,
    }:
    pkgs.rustPlatform.buildRustPackage {
      inherit pname version src cargoRoot;
      allowSubstitutes = false;
      preferLocalBuild = true;
      # Keep both src/ and companion/src/: each crate imports sibling files.
      sourceRoot = "flere-${version}";
      buildAndTestSubdir = cargoRoot;
      cargoDeps = combinedDeps lockFile;
      nativeBuildInputs = [ pkgs.python3 ];
      nativeCheckInputs = with pkgs; [ git python3 bashInteractive zsh vim neovim coreutils ncurses ];
      # The core's src/os.rs links zlib directly for pixel compression.
      buildInputs = lib.optionals (pname == "flere") [ pkgs.zlib ];
      cargoBuildFlags = [
        "--locked"
        "--bin"
        pname
      ];

      # NixOS does not provide this FHS path. Retain an absolute, trusted helper.
      postPatch = ''
        cmp Cargo.lock ${./locks/core-Cargo.lock}
        cmp companion/Cargo.lock ${./locks/companion-Cargo.lock}
        ${pkgs.python3}/bin/python3 ${./test-support.py} . ${fixtureTools} > nix-test-fixtures.json
        cat nix-test-fixtures.json
        substituteInPlace \
          src/install/package.rs \
          companion/src/bootstrap/package.rs \
          companion/src/update.rs \
          --replace-fail '/usr/bin/sha256sum' '${pkgs.coreutils}/bin/sha256sum'
      '';

      # Only synthetic home-cache state; both full suites keep their assertions,
      # timeouts and actual nested companion build. No network Cargo resolution.
      preConfigure = ''
        export HOME="$TMPDIR/h"
        export XDG_CACHE_HOME="$HOME/.cache" XDG_CONFIG_HOME="$HOME/.config"
        export XDG_DATA_HOME="$HOME/.local/share" XDG_STATE_HOME="$HOME/.local/state"
        mkdir -p "$HOME" "$XDG_CACHE_HOME" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_STATE_HOME"
        chmod 700 "$HOME"
        export SHELL=${pkgs.bash}/bin/sh TERM=xterm-256color
        export CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=2
      '';
      doCheck = true;
      checkType = "debug";
      dontUseCargoParallelTests = true;
      cargoTestFlags = [ "--locked" "--all-targets" ];
      checkFlags = [ "--test-threads=1" "--nocapture" ];
      preCheck = ''
        test -c /dev/ptmx
        test "''${#HOME}" -le 20
        for tool in git python3 bash zsh vim nvim; do command -v "$tool"; done
      '';
      postCheck = ''
        cmp Cargo.lock ${./locks/core-Cargo.lock}
        cmp companion/Cargo.lock ${./locks/companion-Cargo.lock}
      '';
      doInstallCheck = true;
      installCheckPhase = ''
        runHook preInstallCheck
        "$out/bin/${pname}" --build-info > build-info.json
        ${pkgs.jq}/bin/jq -e -s \
          --arg component '${pname}' --arg version '${version}' \
          'length == 1 and (.[0] | .schema_version == 1 and .component == $component and .package_version == $version)' \
          build-info.json \
          > /dev/null
        cat build-info.json
        runHook postInstallCheck
      '';

      postInstall = ''
        install -Dm644 LICENSE "$out/share/licenses/${pname}/LICENSE"
        install -Dm644 src/assets/fonts/OFL.txt "$out/share/licenses/${pname}/OFL.txt"
        install -Dm644 src/assets/fonts/LICENSE-Nerd-Fonts "$out/share/licenses/${pname}/LICENSE-Nerd-Fonts"
      '';

      meta = {
        inherit description;
        homepage = "https://github.com/robert-cronin/flere";
        license = [
          lib.licenses.mit
          lib.licenses.ofl
        ];
        mainProgram = pname;
        # Proposed first target, not a claim of completed Nix validation.
        platforms = [ "x86_64-linux" ];
      };
    };
in
assert lib.assertMsg (lib.versionAtLeast pkgs.rustc.version "1.98.0")
  "Flere v0.3.3 requires Rust 1.98 or newer; provide a compatible nixpkgs package set.";
{
  flere = mkPackage {
    pname = "flere";
    description = "Terminal workbench with a custom UI";
    cargoRoot = ".";
    lockFile = ./locks/core-Cargo.lock;
  };

  flere-connect = mkPackage {
    pname = "flere-connect";
    description = "Optional local OpenSSH and clipboard companion for Flere";
    cargoRoot = "companion";
    lockFile = ./locks/companion-Cargo.lock;
  };
}
