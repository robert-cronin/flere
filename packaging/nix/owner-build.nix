# Exact public development source; targeted installed-owner proof, not a release.
{ pkgs ? import <nixpkgs> { }, source, sourceArchive }:
let
  version = "0.3.4";
  # The historical upgrade remains 0.3.3 -> 0.3.4 when default.nix advances.
  previousSource = pkgs.fetchurl {
    url = "https://github.com/robert-cronin/flere/releases/download/v0.3.3/flere-0.3.3-source.tar.gz";
    hash = "sha256-Rhu+Hj+ogCfC6nGRw0rb1+zqrb9vCRonB2mxzcCtap4=";
  };
  package = pname: subdir: lock:
    pkgs.rustPlatform.buildRustPackage {
      inherit pname version;
      # Materialize the verified archive as a store input for the sandbox.
      src = builtins.path {
        path = sourceArchive;
        name = "flere-source.tar";
      };
      sourceRoot = "flere-source";
      cargoRoot = subdir;
      buildAndTestSubdir = subdir;
      cargoDeps = pkgs.rustPlatform.importCargoLock {
        lockFile = builtins.path {
          path = source + "/" + lock;
          name = "${pname}-Cargo.lock";
        };
      };
      allowSubstitutes = false;
      preferLocalBuild = true;
      buildInputs = pkgs.lib.optionals (pname == "flere") [ pkgs.zlib ];
      cargoBuildFlags = [ "--locked" "--bin" pname ];
      # This separate acceptance job does not duplicate the full Rust suites.
      doCheck = false;
      postPatch = ''
        substituteInPlace \
          src/install/package.rs \
          companion/src/bootstrap/package.rs \
          companion/src/update.rs \
          --replace-fail '/usr/bin/sha256sum' '${pkgs.coreutils}/bin/sha256sum'
      '';
      preConfigure = ''
        export HOME="$TMPDIR/h"
        mkdir -m700 -p "$HOME"
        export CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=2
      '';
      postInstall = ''
        install -Dm644 LICENSE "$out/share/licenses/${pname}/LICENSE"
        install -Dm644 src/assets/fonts/OFL.txt "$out/share/licenses/${pname}/OFL.txt"
        install -Dm644 src/assets/fonts/LICENSE-Nerd-Fonts "$out/share/licenses/${pname}/LICENSE-Nerd-Fonts"
      '';
      meta.platforms = [ "x86_64-linux" ];
    };
  previous = current: lockFile: current.overrideAttrs (old: {
    version = "0.3.3";
    src = previousSource;
    sourceRoot = "flere-0.3.3";
    cargoDeps = pkgs.rustPlatform.importCargoLock { inherit lockFile; };
    patches = [ ./patches/wrapped-path-delimiter.patch ];
    postPatch = ''
      cmp Cargo.lock ${./locks/previous-core-Cargo.lock}
      cmp companion/Cargo.lock ${./locks/previous-companion-Cargo.lock}
      ${old.postPatch}
    '';
    # Keep the earlier pair's versioned install check without repeating suites.
    doInstallCheck = true;
    installCheckPhase = ''
      runHook preInstallCheck
      "$out/bin/${old.pname}" --build-info > build-info.json
      ${pkgs.jq}/bin/jq -e -s --arg component '${old.pname}' \
        'length == 1 and (.[0] | .schema_version == 1 and .component == $component and .package_version == "0.3.3")' \
        build-info.json > /dev/null
      cat build-info.json
      runHook postInstallCheck
    '';
  });
  core = package "flere" "." "Cargo.lock";
  companion = package "flere-connect" "companion" "companion/Cargo.lock";
in {
  # Separate upgrade fixtures, not substitutes for the recorded full-suite outputs.
  # Keep the published source/patch, install checks and ordinary package names.
  previous-flere = previous core ./locks/previous-core-Cargo.lock;
  previous-flere-connect = previous companion ./locks/previous-companion-Cargo.lock;
  previous-source = previousSource;
  flere = core;
  flere-connect = companion;
}
