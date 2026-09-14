# Sourced by a Nix derivation with two importCargoLock outputs and the original
# selected lock. Conflicting crate identities or source configurations fail.
set -eu
mkdir -p "$out/.cargo"
cmp "$core_deps/.cargo/config.toml" "$companion_deps/.cargo/config.toml"
cp "$core_deps/.cargo/config.toml" "$out/.cargo/config.toml"
cp "$selected_lock" "$out/Cargo.lock"
for vendor in "$core_deps" "$companion_deps"; do
  for crate in "$vendor"/*; do
    name=$(basename "$crate")
    [ "$name" != Cargo.lock ] || continue
    [ -d "$crate" ] || exit 1
    if [ -e "$out/$name" ]; then
      # Equal name/version must identify the same checksum-derived store input.
      [ "$(realpath "$out/$name")" = "$(realpath "$crate")" ] || exit 1
    else
      ln -s "$(realpath "$crate")" "$out/$name"
    fi
  done
done
