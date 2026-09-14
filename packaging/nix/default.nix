# Draft package expressions; see README.md for the validation limits.
{ pkgs ? import <nixpkgs> { } }:

let
  inherit (pkgs) lib;
  version = "0.3.0";
  src = pkgs.fetchurl {
    url = "https://github.com/robert-cronin/flere/releases/download/v${version}/flere-${version}-source.tar.gz";
    # SHA-256 of the compressed release archive, not an unpacked/NAR hash.
    hash = "sha256-XMx0dlr5iuOPUb4hcJ4+cDMS694+rbWdW0z4rb0x6U8=";
  };

  mkPackage =
    {
      pname,
      description,
      cargoRoot,
      lockFile,
    }:
    pkgs.rustPlatform.buildRustPackage {
      inherit pname version src cargoRoot;
      # Keep both src/ and companion/src/: each crate imports sibling files.
      sourceRoot = "flere-${version}";
      buildAndTestSubdir = cargoRoot;
      cargoLock = { inherit lockFile; };
      # The core's src/os.rs links zlib directly for pixel compression.
      buildInputs = lib.optionals (pname == "flere") [ pkgs.zlib ];
      cargoBuildFlags = [
        "--locked"
        "--bin"
        pname
      ];

      # NixOS does not provide this FHS path. Retain an absolute, trusted helper.
      postPatch = ''
        substituteInPlace \
          src/install/package.rs \
          companion/src/bootstrap/package.rs \
          companion/src/update.rs \
          --replace-fail '/usr/bin/sha256sum' '${pkgs.coreutils}/bin/sha256sum'
      '';

      # The upstream suite needs home-cache state, PTYs and FHS fixture tools.
      # A Nix sandbox adaptation and runtime acceptance are still pending.
      doCheck = false;
      doInstallCheck = true;
      installCheckPhase = ''
        runHook preInstallCheck
        "$out/bin/${pname}" --build-info > build-info.json
        ${pkgs.jq}/bin/jq -e -s \
          --arg component '${pname}' --arg version '${version}' \
          'length == 1 and (.[0] | .schema_version == 1 and .component == $component and .package_version == $version)' \
          build-info.json \
          > /dev/null
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
  "Flere v0.3.0 requires Rust 1.98 or newer; provide a compatible nixpkgs package set.";
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
