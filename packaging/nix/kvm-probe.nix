{ pkgs ? import <nixpkgs> { }, runIdentity, hostNetworkNamespace }:
pkgs.runCommand "flere-nixos-kvm-prerequisite" {
  allowSubstitutes = false;
  preferLocalBuild = true;
  inherit runIdentity hostNetworkNamespace;
  # Intentionally no requiredSystemFeatures=["kvm"]: inspect the actual normal
  # build-user capability without queueing an unschedulable VM test or lying
  # about system-features. No extra sandbox device/path mappings are added.
} ''
  mkdir "$out"
  ${pkgs.coreutils}/bin/timeout --signal=TERM --kill-after=2s 15s \
    ${pkgs.python3}/bin/python3 -I -B ${./kvm-probe.py} \
    --scope sandbox --run-identity "$runIdentity" \
    --host-network-namespace "$hostNetworkNamespace" > "$out/probe.json"
''
