# Exact public development source; targeted installed-owner proof, not a release.
{ pkgs ? import <nixpkgs> { }, source, sourceArchive }:
let
  version = "0.3.4";
  package = pname: subdir: lock:
    pkgs.rustPlatform.buildRustPackage {
      inherit pname version;
      src = builtins.toPath sourceArchive;
      sourceRoot = "flere-source";
      cargoRoot = subdir;
      buildAndTestSubdir = subdir;
      cargoDeps = pkgs.rustPlatform.importCargoLock {
        lockFile = builtins.toPath (source + "/" + lock);
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
in {
  flere = package "flere" "." "Cargo.lock";
  flere-connect = package "flere-connect" "companion" "companion/Cargo.lock";
}
