{ pkgs ? import <nixpkgs> { }, hostNetworkNamespace }:
pkgs.runCommand "flere-required-sandbox" {
  allowSubstitutes = false;
  preferLocalBuild = true;
  inherit hostNetworkNamespace;
} ''
  current="$(${pkgs.coreutils}/bin/readlink /proc/self/ns/net)"
  test -n "$current"
  test "$current" != "$hostNetworkNamespace"
  test -c /dev/ptmx
  mkdir "$out"
  printf 'required sandbox: isolated network namespace and PTY device present\n' > "$out/result"
''
