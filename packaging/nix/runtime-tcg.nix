# Explicit TCG/emulated NixOS runtime acceptance, never KVM/native acceptance.
{ pkgs ? import <nixpkgs> { }, source, sourceArchive, runIdentity }:
let
  core = (import ./owner-build.nix { inherit pkgs source sourceArchive; }).flere;
  smoke = builtins.path { path = ./runtime-smoke.py; name = "flere-runtime-smoke.py"; };
  helper = builtins.path { path = ./owner-probe.py; name = "flere-owner-runtime.py"; };
  command = pkgs.lib.escapeShellArgs [
    "${pkgs.python3}/bin/python3" "-I" "-B" "${smoke}"
    "--runtime-helper" "${helper}"
    "--core" "${core}/bin/flere"
    "--shell" "${pkgs.bashInteractive}/bin/bash"
    "--editor" "${pkgs.vim}/bin/vim"
    "--git" "${pkgs.git}/bin/git"
    "--output" "/home/tester/.cache/flere/tmp/runtime.json"
    "--run-identity" runIdentity
  ];
  test = pkgs.testers.runNixOSTest {
    name = "flere-tcg";
    requiredFeatures.kvm = false;
    qemu.forceAccel = false;
    globalTimeout = 600;
    enableDebugHook = false;
    sshBackdoor.enable = false;
    nodes.machine = { ... }: {
      virtualisation = {
        cores = 2;
        memorySize = 2048;
        diskSize = 2048;
        graphics = false;
        # qemu-common initially emits kvm:tcg; this explicit later setting
        # selects only TCG, as in the pinned NixOS optee test. Verify it below.
        qemu.options = [ "-machine accel=tcg" ];
      };
      users.users.tester = {
        isNormalUser = true;
        createHome = true;
        home = "/home/tester";
        shell = pkgs.bashInteractive;
      };
      environment.systemPackages = [ core pkgs.bashInteractive pkgs.vim pkgs.git pkgs.python3 ];
      services.openssh.enable = false;
      system.stateVersion = "26.05";
    };
    testScript = ''
      import datetime as dt
      import json
      import os
      from pathlib import Path
      import shlex

      start_all()
      assert machine.process is not None
      raw = Path(f"/proc/{machine.process.pid}/cmdline").read_bytes()
      assert len(raw) <= 65536
      argv = raw.rstrip(b"\0").decode().split("\0")
      accelerators = [part.split("=", 1)[1]
                      for i, arg in enumerate(argv[:-1]) if arg == "-machine"
                      for part in argv[i + 1].split(",") if part.startswith("accel=")]
      assert accelerators and accelerators[-1] == "tcg", argv
      assert "-enable-kvm" not in argv and "-accel" not in argv, argv
      monitor = machine.send_monitor_command("info kvm")
      assert "kvm support: disabled" in monitor.lower(), monitor
      acceleration = {"execution": "TCG/emulated NixOS", "run_identity": ${builtins.toJSON runIdentity},
                      "qemu_pid": machine.process.pid, "qemu_argv": argv,
                      "monitor_info_kvm": monitor, "kvm_enabled": False}
      print(json.dumps({"acceleration": acceleration}, sort_keys=True))
      machine.wait_for_unit("multi-user.target", timeout=dt.timedelta(seconds=300))
      try:
          machine.succeed("su -- tester -c " + shlex.quote(${builtins.toJSON command}),
                          timeout=dt.timedelta(seconds=240))
      finally:
          exists, _ = machine.execute("test -f /home/tester/.cache/flere/tmp/runtime.json",
                                      timeout=dt.timedelta(seconds=10))
          if exists == 0:
              machine.succeed("test $(stat -c %s /home/tester/.cache/flere/tmp/runtime.json) -le 65536")
              guest_receipt = json.loads(machine.succeed("cat /home/tester/.cache/flere/tmp/runtime.json"))
              print(json.dumps({"guest_receipt": guest_receipt}, sort_keys=True))
              machine.copy_from_vm("/home/tester/.cache/flere/tmp/runtime.json")
      machine.shutdown()
      assert machine.process.poll() == 0
      acceleration["qemu_exit"] = machine.process.returncode
      (Path(os.environ["out"]) / "acceleration.json").write_text(json.dumps(acceleration, indent=2, sort_keys=True) + "\n")
    '';
  };
in {
  inherit core;
  test = test.overrideTestDerivation (_: {
    allowSubstitutes = false;
    preferLocalBuild = true;
    inherit runIdentity;
  });
}
