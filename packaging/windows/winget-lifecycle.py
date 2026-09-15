#!/usr/bin/env python3
"""Normal first WinGet portable install/remove on one disposable hosted Windows VM.

Fixed retained inputs; the default preserves the earlier 422 lifecycle.
The selected candidates also check installed ownership with a synthetic
profile. The explicit upgrade mode installs published 0.3.4 bytes before the
reviewed 0.3.5 candidate. No public download-route, UI, coordinated-refusal or SSH claim.
The community source is contacted normally. Upgrade-only VM provisioning removes
the observed default Store source before the protected source baseline; that source
set remains until VM teardown. LocalManifestFiles changes use normal settings
commands with readback and original-state restoration.
"""
import argparse
import base64
import copy
import importlib.util
import io
import json
import ntpath
import os
from pathlib import Path, PureWindowsPath
import platform
import re
import shutil
import subprocess
import sys
import time
import traceback
import zipfile

sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location("choco_fixture", Path(__file__).with_name("chocolatey-lifecycle.py"))
base = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(base)
candidate, require, sha = base.candidate, base.require, base.sha
PACKAGE, VERSION = "RobertCronin.FlereConnect", "0.3.4"
PRODUCT_CODE = PACKAGE + "__DefaultSource"
WINGET_VERSION = "v1.11.510"
SOURCE_URL = "https://cdn.winget.microsoft.com/cache"
SOURCE_ID = "Microsoft.Winget.Source_8wekyb3d8bbwe"
SETTING_SCHEMA = "https://aka.ms/winget-settings-export.schema.json"
MANIFESTS = {
    PACKAGE + ".installer.yaml": "e780fb0839499e3ad196410a552c667877ca6f5779126fb81883640710fabe82",
    PACKAGE + ".locale.en-US.yaml": "782cdfeff696c8fc19920270acef8b7cc5ab6d950c5eab5bbb9b282491ddd8a9",
    PACKAGE + ".yaml": "4a12e5e84b053b265931e1f5970dce9134f4fc89893c9ffd260850a86d78e049",
}
# Fixed reviewed artifact identities; never accept a caller-provided URL or tuple.
DEFAULT_INPUT = base.DEFAULT_INPUT
LEGACY_INPUT = dict(base.LEGACY_INPUT, winget=MANIFESTS)
CURRENT_INPUT = {'name': 'candidate-9a9183c',
 'run': 34931604932,
 'artifact': 10380984686,
 'artifact_bytes': 1876464,
 'artifact_name': 'windows-candidate-34931604932-1',
 'workflow': '9a9183c7708666e20d8790fb149fbd75204e0a85',
 'artifact_sha': 'c6110bfe3348a936039831d93a0f17aebb1bc11c34509d96b70bf6907e0e7d0d',
 'product': '9a9183c7708666e20d8790fb149fbd75204e0a85',
 'source': '20502c15ff90b9fb78edc20ae7573bbf6b441397bc6b8b6728ba402254b96715',
 'zip_sha': '101787975330d19a73828397ef314d0563cc98634e74cec1899fb6b991831da7',
 'zip_bytes': 1834036,
 'payload_sha': '5505cd13c6f55024588a9601f75aa7dfce12c456b8c71bd550dcdbe794d57b83',
 'manifest_sha': 'de16659f48453d10b108692823240c96ac2d036aa9fcdbaf54954c021ac614ac',
 'nuspec_sha': '8ba0dd77f894cdfebfa093019dad91e3bd50aafe7c1f180a2665c0a5bc6dc681',
 'script_sha': '1aaff6d0e29adaf4aeb4f2cb31dd18b12a68532a1db573574059cdb2a9724542',
 'nupkg_sha': '2eba4e4aa4270ee257c31f46d13a00251f1a74cf8b50f49da55be0779eb2974f',
 'receipt_sha': 'ef098422b127cafa7fa527bb0c994f7669757738ef0948a105900030bc38d9c8',
 'source_entries': 384,
 'artifact_entries': 33,
 'winget': {'RobertCronin.FlereConnect.installer.yaml': '0c788459ef4645a3b12fa57e313da0f84a140133ec377fb3591ba2c9a25fe61b',
            'RobertCronin.FlereConnect.locale.en-US.yaml': '782cdfeff696c8fc19920270acef8b7cc5ab6d950c5eab5bbb9b282491ddd8a9',
            'RobertCronin.FlereConnect.yaml': '4a12e5e84b053b265931e1f5970dce9134f4fc89893c9ffd260850a86d78e049'},
 'owner_check': True}
# The corrected profile resolver has its own immutable candidate input.
PROFILE_INPUT = {'name': 'candidate-9d53f96',
 'run': 34935547435,
 'artifact': 10382474094,
 'artifact_bytes': 1877630,
 'artifact_name': 'windows-candidate-34935547435-1',
 'workflow': '9d53f968d306e1ff95f4d1c8af3a74db45cb6106',
 'artifact_sha': '90b12d0a6de1d3780d62301b381368e2c9b63f4d7d47cfa1dd70d5c7a2ce6ab6',
 'product': '9d53f968d306e1ff95f4d1c8af3a74db45cb6106',
 'source': 'b442909fdf817d7c493e57e5c61182cf36b5051a4764ab98db567dc9ab84e654',
 'zip_sha': '1d8e5e66620c9dd62ba8f5ebdeb7d7bb8f7909438c1be4ccfa4dc1256439dee0',
 'zip_bytes': 1835221,
 'payload_sha': 'd04db345069cc0aed0e7a274749c587c4a657b7cacc8f919aa89e8a0b0bbe838',
 'manifest_sha': '3bc3e03f9e9370ba31f12b4e82ec59c9a2a902a27f0bb33f8d1ae946935a5479',
 'nuspec_sha': '8ba0dd77f894cdfebfa093019dad91e3bd50aafe7c1f180a2665c0a5bc6dc681',
 'script_sha': '424626b1efad85f19d4f324723f52946a286147f2f94c1e0a0a3565697d0f337',
 'nupkg_sha': '73b617a35b52b57d6a347426dee44daa0c7422b2aab8649e098d8402f88b0120',
 'receipt_sha': '87750caffc2c88dc248a14d7a1f58a68d3afb23b1f79632382648455d2dd9d4b',
 'source_entries': 384,
 'artifact_entries': 33,
 'winget': {'RobertCronin.FlereConnect.installer.yaml': 'd03839f6e99d56743b6871759739db3a13d6c924f40e42ef7f6bc7bfda707365',
            'RobertCronin.FlereConnect.locale.en-US.yaml': '782cdfeff696c8fc19920270acef8b7cc5ab6d950c5eab5bbb9b282491ddd8a9',
            'RobertCronin.FlereConnect.yaml': '4a12e5e84b053b265931e1f5970dce9134f4fc89893c9ffd260850a86d78e049'},
 'owner_check': True}
# Independently verified native035 candidate; public034 remains a separate baseline.
CANDIDATE035_INPUT = {'name': 'candidate-7e43fa1',
 'version': '0.3.5',
 'run': 34938451094,
 'artifact': 10383739920,
 'artifact_bytes': 1896364,
 'artifact_name': 'windows-candidate-34938451094-1',
 'workflow': '7e43fa11d8cf17d81388c1be94da083a0c58a760',
 'artifact_sha': 'a53eac44cff573aad1ad05aa7a0c61ffc10ad6b2357d4048cf682355e276c412',
 'product': '7e43fa11d8cf17d81388c1be94da083a0c58a760',
 'source': '503875747896c39552171bc380c74fc108546bbf287137ec34c8525f8f8cd26e',
 'zip_sha': 'fc3e8fab4e951d596b57300bdf1bf67e98543ea416bff1e0fa89492238229809',
 'zip_bytes': 1853778,
 'payload_sha': 'fabd2a0fad3e3ff97c34f020390b1730b3fecdc4125f730200e4eb25a8dc1872',
 'manifest_sha': '8350f3cfd9a1853ecc5499cf7260b0e1ce556ee6942dac1ef3081a3c6ece7721',
 'nuspec_sha': '7ef58bc1b5b39271e9fce4b3ff635df3eca59ab6d880eca7c971b2e96b4fcf69',
 'script_sha': '6a3bb6c15856204c22adb7fb4e7fc4a2d898b2a0c2afda0ec7272d47f1b1e459',
 'nupkg_sha': '95548c6d1100f0406e0b4c32aa3ea4884c63b29f82f69a31c98af622e82b6955',
 'receipt_sha': '7bcf273f505a208e59440ee69f2f1c5a14be7d5e306d65f3246bc5ba8f921387',
 'source_entries': 388,
 'artifact_entries': 33,
 'owner_check': True,
 'winget': {'RobertCronin.FlereConnect.installer.yaml': '040686c13b7ef085c0d7f0d73eb48006ab7dd3d6a316ba985dc52491790bc18d',
            'RobertCronin.FlereConnect.locale.en-US.yaml': '1fff4b3405f83de5dcab367b7e31ac261b0f21546ea6921df34c12da10ee8bb9',
            'RobertCronin.FlereConnect.yaml': 'e9019088daa9e9e4d016b96f0ab6ff648b064ad164375052b32694c93bc32180'}}
INPUTS = {value["name"]: value for value in (LEGACY_INPUT, CURRENT_INPUT, PROFILE_INPUT, CANDIDATE035_INPUT)}


# Exact release producer archive; its ZIP matches immutable public v0.3.4.
# owner_check selects the candidate proof schema here, not a baseline owner call.
PUBLIC034_INPUT = {'name': 'published-0.3.4',
 'version': '0.3.4',
 'run': 34930826557,
 'artifact': 10381936022,
 'artifact_bytes': 1864914,
 'artifact_name': 'release-windows-34930826557-1',
 'workflow': '32108e352f4a51d809b1e5ed64cc9dba3bc0be88',
 'artifact_sha': 'd87eddb9960995a203887cf5c5d5260034c4059867edf43bf88fb69435159ad6',
 'product': '32108e352f4a51d809b1e5ed64cc9dba3bc0be88',
 'source': '9a801df30c3e71a1c61bc04cf3ed15b1f8574ea46b9bb2c75561c3850ff3bf35',
 'zip_sha': '71fd96f768894d849c4773add52bd0e3131e032c77640a02aa1c4af36e5001db',
 'zip_bytes': 1822693,
 'payload_sha': 'ccf4985a7df7174206cb6ca9f6df21dd0619a4402f2f8f11736a7b0fd940b302',
 'manifest_sha': 'a6038e1f993b13ef953b7818e77fae639b228546cf2d88325a30dcbd8024ff31',
 'nuspec_sha': '8ba0dd77f894cdfebfa093019dad91e3bd50aafe7c1f180a2665c0a5bc6dc681',
 'script_sha': '6780e4a3ec300809ec52442bfe2142e2e19533772f8d4521811b317ee37c42fc',
 'nupkg_sha': '95cd639010892eea109247ebc651aed1f7811fd1fa1080d8cc6d8299276ace9b',
 'receipt_sha': '20da0ed83a12bb0c0ade783a5363e6481e095c7e23c9bd2606e7748302a4ea91',
 'source_entries': 383,
 'artifact_entries': 33,
 'winget': {'RobertCronin.FlereConnect.installer.yaml': '0e9f970db74933514c6bef0b24715f2bb906b73d9a6864e3538cb0b9ed172225',
            'RobertCronin.FlereConnect.locale.en-US.yaml': '782cdfeff696c8fc19920270acef8b7cc5ab6d950c5eab5bbb9b282491ddd8a9',
            'RobertCronin.FlereConnect.yaml': '4a12e5e84b053b265931e1f5970dce9134f4fc89893c9ffd260850a86d78e049'},
 'owner_check': True}


def upgrade_baseline(selected, requested):
    require(type(requested) is bool, "explicit upgrade mode must be boolean")
    if not requested:
        return None
    require(base.input_version(selected) == "0.3.5" and selected["owner_check"] is True,
            "upgrade requires the fixed reviewed 0.3.5 candidate; same-version reinstall is not an upgrade")
    return copy.deepcopy(PUBLIC034_INPUT)


def selection(name=DEFAULT_INPUT):
    require(name in INPUTS, "only an exact reviewed retained input may be selected")
    return copy.deepcopy(INPUTS[name])


SOURCE_TITLE = b"The `winget` source requires that you view the following agreements before using."
SOURCE_PROMPT = b"Do you agree to all the source agreements terms?"
OPTIONS = b"[Y] Yes  [N] No: "
ARP = r"Software\Microsoft\Windows\CurrentVersion\Uninstall"


GUIDANCE = ("Local WinGet: This companion is installed as a WinGet portable package. Use WinGet "
            "with the next reviewed Flere manifest/package, then reopen the companion. In-app Apply is disabled.")


def windows_path(value):
    require(isinstance(value, str) and not any(ord(c) < 32 or ord(c) == 127 for c in value), "unsafe Windows path")
    path = PureWindowsPath(value.removeprefix("\\\\?\\"))
    require(path.is_absolute() and ".." not in path.parts, "absolute Windows path required")
    return path


def verify_installed_owner(value, manifest, executable):
    require(isinstance(value, dict) and {k: v for k, v in value.items() if k != "ownership"} == {
        "schema_version": 1, "running_build": manifest["build"], "installation": None,
        "other_frontends": "untracked"}, "installed status fields/full build differ")
    owner = value.get("ownership")
    require(isinstance(owner, dict) and windows_path(owner.get("executable")) == windows_path(str(executable)),
            "owner executable is not the selected installed alias payload")
    require(owner == {"kind": "manager", "executable": owner["executable"],
                      "sha256": manifest["payload"]["sha256"], "attempt": None, "guidance": GUIDANCE},
            "verified WinGet owner/guidance differs")
    return owner


def verify_synthetic_override(environment, executable):
    home = windows_path(environment["HOME"])
    require(windows_path(environment["USERPROFILE"]) == home, "fixture profile differs")
    actual_appdata = windows_path(str(executable)).parents[4]
    require(str(actual_appdata).lower().endswith("\\appdata\\local"), "installed default user package path differs")
    for key in ("LOCALAPPDATA", "APPDATA", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME", "TEMP", "TMP"):
        require(windows_path(environment[key]).is_relative_to(home), "fixture environment escaped: " + key)
    require(windows_path(environment["LOCALAPPDATA"]) != actual_appdata,
            "test must override LOCALAPPDATA instead of using the installed manager root")
    return {key: environment[key] for key in ("HOME", "USERPROFILE", "LOCALAPPDATA", "APPDATA",
                                            "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME", "TEMP", "TMP")}


def verify_powershell_initialization(before, after):
    base.verify_tool_initialization(before, after)
    require("temp/chocolatey" not in after.keys() - before.keys(),
            "WinGet owner checks do not initialize Chocolatey")


def clean_console(data):
    # Interpretation only; retain original bytes. Only ordinary SGR/progress
    # cursor sequences from the known manager are ignored for text matching.
    return re.sub(rb"\x1b\[[0-9;?]*[mGKJABCDHf]", b"", data).replace(b"\r", b"")


def prompt_ready(data, answered, allow_source):
    text = clean_console(data)
    titles = re.findall(rb"The `[^`\n]+` source requires that you view the following agreements before using\.", text)
    require(all(title == SOURCE_TITLE for title in titles) and len(titles) <= 1,
            "unexpected source agreement context")
    require(b"The publisher requires that you view" not in text and b"Do you want to" not in text,
            "unexpected package or other prompt")
    count = text.count(OPTIONS)
    require(count <= 1 and text.count(SOURCE_PROMPT) <= 1, "repeated confirmation prompt")
    if count:
        require(allow_source and titles == [SOURCE_TITLE] and text.count(SOURCE_PROMPT) == 1
                and text.index(SOURCE_TITLE) < text.index(SOURCE_PROMPT) < text.index(OPTIONS),
                "prompt lacks reviewed community-source context")
        # Retain the actual community terms; Store market consent is outside this fixture.
        require(b"2-letter geographic region" not in text, "unexpected source market agreement")
    return count == 1 and not answered


def source_export(data):
    decoder = json.JSONDecoder(); text = data.decode("utf-8-sig"); rows = []
    while text.strip():
        value, end = decoder.raw_decode(text.lstrip())
        require(isinstance(value, dict), "source export entry is not an object")
        rows.append(value); text = text.lstrip()[end:]
        require(len(rows) <= 8, "source inventory exceeds bound")
    require(rows and len({row.get("Name") for row in rows}) == len(rows), "duplicate/empty source inventory")
    matches = [row for row in rows if row.get("Name") == "winget"]
    require(len(matches) == 1, "normal community source missing")
    source = matches[0]
    require(source.get("Arg") == SOURCE_URL and source.get("Type") == "Microsoft.PreIndexed.Package"
            and source.get("Identifier") == SOURCE_ID and source.get("Explicit") is False,
            "community source identity differs")
    return sorted(rows, key=lambda row: row["Name"])


def upgrade_source_baseline(original):
    """Only the observed default Store source may be removed from this test VM."""
    store = {"Name":"msstore", "Arg":"https://storeedgefd.dsx.mp.microsoft.com/v9.0",
             "Type":"Microsoft.Rest", "Identifier":"StoreEdgeFD", "Explicit":False,
             "Data":"", "TrustLevel":["Trusted"]}
    community = {"Name":"winget", "Arg":SOURCE_URL, "Type":"Microsoft.PreIndexed.Package",
                 "Identifier":SOURCE_ID, "Explicit":False, "Data":SOURCE_ID,
                 "TrustLevel":["Trusted","StoreOrigin"]}
    require(original == [store, community], "upgrade source provisioning requires the observed default sources")
    return [community]


def setting_value(value):
    require(isinstance(value, dict) and value.get("$schema") == SETTING_SCHEMA
            and isinstance(value.get("userSettingsFile"), str) and value["userSettingsFile"]
            and isinstance(value.get("adminSettings"), dict)
            and type(value["adminSettings"].get("LocalManifestFiles")) is bool,
            "unexpected settings export")
    return value["adminSettings"]["LocalManifestFiles"]


def verify_settings(before, after, enabled):
    setting_value(before); setting_value(after)
    expected = copy.deepcopy(before); expected["adminSettings"]["LocalManifestFiles"] = enabled
    require(type(enabled) is bool and after == expected, "settings changed beyond LocalManifestFiles")


def verify_user_settings(value):
    require(isinstance(value, dict), "user settings must be an object")
    for section, key, default in (("installBehavior", "portablePackageUserRoot", ""),
                                 ("installBehavior", "portablePackageMachineRoot", ""),
                                 ("interactivity", "disable", False),
                                 ("uninstallBehavior", "purgePortablePackage", False)):
        group = value.get(section, {})
        require(isinstance(group, dict) and group.get(key, default) == default
                and type(group.get(key, default)) is type(default), "unsupported custom setting: " + key)


def local_manifest(name, original, port, selected=LEGACY_INPUT):
    require(type(port) is int and 49152 <= port <= 65535, "owned high loopback port required")
    require(name in selected["winget"] and sha(original) == selected["winget"][name], "frozen manifest differs")
    version, zip_name = base.input_version(selected), base.zip_name(selected)
    public_url = f"https://github.com/{base.REPO}/releases/download/v{version}/{zip_name}".encode()
    updated = original
    if name.endswith(".installer.yaml"):
        require(original.count(public_url) == 1, "one installer URL required")
        local = f"http://127.0.0.1:{port}/{zip_name}".encode()
        updated = original.replace(public_url, local)
        require(updated.replace(local, public_url) == original, "non-URL manifest change")
    return updated


def local_manifests(archive_bytes, destination, port, selected=LEGACY_INPUT):
    require(len(archive_bytes) == selected["artifact_bytes"] and sha(archive_bytes) == selected["artifact_sha"],
            "recipe artifact differs")
    original_prefix = f"windows/winget/manifests/r/RobertCronin/FlereConnect/{base.input_version(selected)}/"
    destination.mkdir()
    changes = {}
    with zipfile.ZipFile(io.BytesIO(archive_bytes)) as archive:
        for name, digest in selected["winget"].items():
            original = archive.read(original_prefix + name)
            updated = local_manifest(name, original, port, selected)
            (destination / name).write_bytes(updated)
            changes[name] = {"original_sha256": digest, "private_sha256": sha(updated)}
    return changes


def own_record(records, *, complete=True, version=VERSION):
    rows = [row for row in records if row["values"].get("WinGetPackageIdentifier") == PACKAGE]
    require(len(rows) == 1, "exactly one active package record required")
    row = rows[0]; values = row["values"]
    require(row["hive"] == "HKCU" and row["view"] == "64"
            and values.get("DisplayVersion") == version and values.get("DisplayName") == "Flere Connect"
            and values.get("WinGetInstallerType") == "portable"
            and values.get("WinGetSourceIdentifier") == "*DefaultSource",
            "active user portable record differs")
    if complete:
        require(values.get("InstallDirectoryCreated") == 1
                and values.get("InstallDirectoryAddedToPath", 0) == 0,
                "portable install incomplete or alias fallback occurred")
    return row


def local_package_args(owned, *, remove=False, version=VERSION):
    # Local-manifest portable installs use the observed default-source product
    # code, not the community catalogue ID. Never execute an ARP command string.
    own_record([owned], complete=False, version=version)
    require(owned["subkey"] == ARP + "\\" + PRODUCT_CODE
            and owned["values"].get("UninstallString") == "winget uninstall --product-code " + PRODUCT_CODE,
            "local portable product code differs")
    selection = (["uninstall", "--product-code", PRODUCT_CODE] if remove else
                 ["list", "--name", owned["values"]["DisplayName"]])
    # Keep the one reviewed community source; source-less commands would open
    # all configured sources. --manifest and --source are mutually exclusive.
    return selection + ["--exact", "--scope", "user", "--source", "winget"]


def verify_inventory(before, after, installed, *, version=VERSION):
    old = {row["key"]: row["sha256"] for row in before}
    new = {row["key"]: row["sha256"] for row in after}
    require(len(old) == len(before) and len(new) == len(after), "duplicate installed record")
    if installed:
        owned = own_record(after, version=version)
        require(owned["key"] not in old, "owned registry entry existed before install")
        del new[owned["key"]]
    require(new == old, "unrelated installed-program inventory changed")


def verify_upgrade_identity(before, after):
    old = own_record([before], version="0.3.4")
    new = own_record([after], version="0.3.5")
    require(old["key"] == new["key"] and old["subkey"] == new["subkey"]
            and windows_path(old["values"]["InstallLocation"]) == windows_path(new["values"]["InstallLocation"]),
            "upgrade changed the exact installed package identity/root")


def verify_removal(value):
    require(all(value[key] is True for key in ("installed_inventory_preserved", "path_preserved", "state_preserved",
                                              "links_preserved", "package_directories_preserved"))
            and value["package_directory_exists"] is False
            and set(value["aliases"]) == {"flere.exe", "flere-connect.exe"}
            and all(alias["link_exists"] is False and alias["resolution"] is None for alias in value["aliases"].values()),
            "normal uninstall/preservation check failed")


def registry_inventory():
    import winreg
    result = []
    for hive_name, hive, view_name, view in (("HKCU", winreg.HKEY_CURRENT_USER, "64", winreg.KEY_WOW64_64KEY),
                                            ("HKLM", winreg.HKEY_LOCAL_MACHINE, "64", winreg.KEY_WOW64_64KEY),
                                            ("HKLM", winreg.HKEY_LOCAL_MACHINE, "32", winreg.KEY_WOW64_32KEY)):
        try:
            parent = winreg.OpenKey(hive, ARP, 0, winreg.KEY_READ | view)
        except FileNotFoundError:
            continue
        with parent:
            count = winreg.QueryInfoKey(parent)[0]; require(count <= 1024, "installed registry bound exceeded")
            names = sorted(winreg.EnumKey(parent, i) for i in range(count))
            for name in names:
                with winreg.OpenKey(parent, name) as child:
                    count_values = winreg.QueryInfoKey(child)[1]
                    require(count_values <= 128, "installed record value bound exceeded")
                    values, typed = {}, {}
                    for index in range(count_values):
                        key, value, kind = winreg.EnumValue(child, index)
                        values[key] = value
                        typed[key] = [kind, {"base64": base64.b64encode(value).decode()} if isinstance(value, bytes) else value]
                    raw = json.dumps(typed, sort_keys=True).encode(); require(len(raw) <= 65536, "installed record too large")
                row = {"key": hive_name + "/" + view_name + "/" + name, "hive": hive_name,
                       "view": view_name, "subkey": ARP + "\\" + name, "sha256": sha(raw)}
                # Export raw values only for the exact own ID; other software stays hashed.
                row["values"] = values if values.get("WinGetPackageIdentifier") == PACKAGE else {}
                require("flere" not in str(values.get("DisplayName", "")).lower() or row["values"],
                        "conflicting existing Flere package")
                result.append(row)
    require(len(result) <= 2048, "installed inventory exceeds bound")
    return sorted(result, key=lambda row: row["key"])


def normal_path():
    import winreg
    parts = []
    for hive, key in ((winreg.HKEY_LOCAL_MACHINE, r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment"),
                       (winreg.HKEY_CURRENT_USER, r"Environment")):
        with winreg.OpenKey(hive, key) as opened:
            try:
                value, kind = winreg.QueryValueEx(opened, "Path")
            except FileNotFoundError:
                continue
        require(kind in (winreg.REG_SZ, winreg.REG_EXPAND_SZ) and isinstance(value, str), "PATH registry type differs")
        parts.append(os.path.expandvars(value))
    return ";".join(parts)


def verify_link_path_baseline(path_value, links, before_links):
    def key(value):
        return ntpath.normpath(value.strip().strip('"')).casefold()
    present = key(str(links)) in {key(value) for value in path_value.split(";") if value.strip()}
    require(not (present and not before_links), "normal uninstall would remove a preexisting empty-links PATH entry")


def link_inventory(root):
    if not root.exists():
        require(not os.path.lexists(root), "dangling portable root")
        return {}
    base.file_record(root)
    paths = sorted(root.iterdir()); require(len(paths) <= 256, "portable links bound exceeded")
    result = {}
    for path in paths:
        st = path.lstat()
        if path.is_symlink():
            result[path.name] = {"link": os.readlink(path), "file_id": st.st_ino, "device": st.st_dev}
        else:
            result[path.name] = base.file_record(path)
    return result


class Run:
    def __init__(self, work, proof, receipt):
        self.work, self.proof, self.receipt = work, proof, receipt
        self.env = base.runtime_env(os.environ)
        self.deadline = time.monotonic() + 900  # Reserve cleanup inside a 20-minute job.

    def save(self):
        (self.proof / "receipt.json").write_text(json.dumps(self.receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    def record(self, name, value):
        raw = json.dumps(value, indent=2, sort_keys=True).encode()
        require(len(raw) <= 1024 * 1024, "record exceeds bound")
        (self.proof / "records" / (name + ".json")).write_bytes(raw + b"\n")

    def command(self, name, argv, *, env=None, seconds=120, maximum=base.MAX_LOG, source_prompt=False, destination=None, accepted=(0,)):
        seconds = min(seconds, self.deadline - time.monotonic()); require(seconds > 0, "lifecycle deadline exhausted")
        log = destination or self.proof / "logs" / (name + ".log")
        errlog = self.proof / "logs" / (name + "-stderr.log")
        row = {"name": name, "argv": list(map(str, argv)), "status": "running", "confirmed_source": False}
        self.receipt["checks"].append(row); self.save(); start = time.monotonic(); failure = None
        with log.open("xb") as out, errlog.open("xb") as err:
            proc = subprocess.Popen(row["argv"], cwd=self.work, env=self.env if env is None else env,
                                    stdin=subprocess.PIPE, stdout=out, stderr=err)
            row["pid"] = proc.pid
            try:
                while proc.poll() is None:
                    require(time.monotonic() - start < seconds, name + " timed out")
                    require(log.stat().st_size <= maximum and errlog.stat().st_size <= base.MAX_LOG, name + " output bound")
                    if destination is None and prompt_ready(log.read_bytes() + errlog.read_bytes(), row["confirmed_source"], source_prompt):
                        require(not any(check["confirmed_source"] for check in self.receipt["checks"]), "source confirmation repeated across commands")
                        proc.stdin.write(b"Y\r\n"); proc.stdin.flush(); row["confirmed_source"] = True
                    time.sleep(0.05)
            except BaseException as error:
                failure = error
            finally:
                row["forced_stop"] = proc.poll() is None
                if row["forced_stop"]:
                    killed = subprocess.run([str(Path(os.environ["SystemRoot"]) / "System32/taskkill.exe"),
                        "/PID", str(proc.pid), "/T", "/F"], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                        stderr=subprocess.DEVNULL, timeout=15)
                    require(killed.returncode == 0, "owned child tree could not be stopped")
                proc.wait(timeout=15); proc.stdin.close()
        for path, bound in ((log, maximum), (errlog, base.MAX_LOG)):
            if path.stat().st_size > bound:
                failure = failure or ValueError(name + " output bound")
                with path.open("r+b") as stream:
                    stream.truncate(bound)
        data, errors = log.read_bytes(), errlog.read_bytes()
        if destination is None and failure is None:
            try:
                require(not prompt_ready(data + errors, row["confirmed_source"], source_prompt), "prompt exited before response")
            except ValueError as error:
                failure = error
        row.update(exit=proc.returncode, seconds=round(time.monotonic()-start, 3),
                   stdout_bytes=len(data), stdout_sha256=sha(data), stderr_bytes=len(errors), stderr_sha256=sha(errors),
                   status="passed" if failure is None and proc.returncode in accepted else "failed")
        self.save()
        if failure:
            raise failure
        require(proc.returncode in accepted, name + " failed; inspect retained log")
        return data

    def ps(self, name, code):
        text = "$ErrorActionPreference='Stop'; " + code
        encoded = base64.b64encode(text.encode("utf-16-le")).decode()
        return json.loads(self.command(name, [self.pwsh, "-NoLogo", "-NoProfile", "-NonInteractive", "-EncodedCommand", encoded]))


def main(output, selected=LEGACY_INPUT, *, upgrade=False):
    baseline = upgrade_baseline(selected, upgrade)
    target_version = base.input_version(selected)
    candidate.hosted(os.environ)
    require(os.name == "nt" and platform.machine().lower() in ("amd64", "x86_64") and sys.version_info >= (3, 12), "native Windows AMD64 required")
    require(subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=candidate.PROJECT, text=True, timeout=15).strip()
            == os.environ["FLERE_WORKFLOW_SHA"], "workflow checkout differs")
    require(output.parent.resolve() == Path.home().resolve()/".cache/flere/tmp" and not output.exists(), "fresh private home-cache output required")
    output.mkdir(); work = output/"work"; proof = output/"proof"
    for path in (work, proof, proof/"logs", proof/"records"):
        path.mkdir()
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as stream:
        stream.write("evidence=" + str(proof) + "\n")
    receipt = {"schema": "flere-winget-lifecycle-v1", "status": "running", "checks": [],
        "workflow_sha": os.environ["FLERE_WORKFLOW_SHA"], "run_id": os.environ["GITHUB_RUN_ID"],
        "run_attempt": os.environ["GITHUB_RUN_ATTEMPT"], "input_selection": selected["name"],
        "installed_owner_check_required": selected["owner_check"], "input_artifact": selected["artifact"],
        "artifact_sha256": selected["artifact_sha"], "product_commit": selected["product"], "source_sha256": selected["source"],
        "zip_sha256": selected["zip_sha"], "limits": ["First private-loopback install/remove; no public URL or upgrade proof.",
        ("Installed owner JSON only; UI/coordinated refusal remains unverified." if selected["owner_check"] else
         "No WinGet detector/update-refusal claim."),
        "No signing, physical UI, clipboard or SSH claim.",
        "Normal Microsoft community-source network access and OS archive malware scanning are enabled."],
        "machine": {"platform": platform.platform(), "image_os": os.environ.get("ImageOS"),
                    "image_version": os.environ.get("ImageVersion"), "python": platform.python_version()}}
    if upgrade:
        receipt["upgrade_from"] = {"version": "0.3.4", "product_commit": baseline["product"],
            "zip_sha256": baseline["zip_sha"], "artifact_sha256": baseline["artifact_sha"]}
        receipt["upgrade_to_version"] = target_version
        receipt["limits"][0] = "Normal local-manifest 0.3.4 to 0.3.5 upgrade; identical public baseline bytes via loopback, not public URL/catalogue acceptance."
    run = Run(work, proof, receipt); mirrors = []; settings_before = None; changed_setting = False
    user_before = sources_before = None
    attempted = False; removed = False; manifests = work/"manifests"; installed_root = None

    def settings(name):
        value = json.loads(run.command(name, [winget, "settings", "export"]))
        setting_value(value); run.record(name, value); return value

    def user_file(name):
        # Do not parse arbitrary files; only the reported standard runner profile settings path.
        path = Path(settings_before["userSettingsFile"])
        require(path.is_absolute() and path.resolve().is_relative_to(local_appdata.resolve()) and path.name == "settings.json",
                "user settings path is outside normal runner LocalAppData")
        if not path.exists():
            require(not os.path.lexists(path), "settings path is dangling")
            # Invalid/missing primary settings can load the backup: reject that ambiguity.
            require(not path.parent.exists() or not list(path.parent.glob("settings.json*")), "backup-only user settings unsupported")
            return {"path": str(path), "present": False}
        value = base.file_record(path); require(value["bytes"] <= 65536, "user settings bound")
        escaped = str(path).replace("'", "''")
        parsed = run.ps(name, "$o=[System.Text.Json.JsonDocumentOptions]::new(); $o.AllowTrailingCommas=$true; "
            "$o.CommentHandling=[System.Text.Json.JsonCommentHandling]::Skip; "
            "$d=[System.Text.Json.JsonDocument]::Parse([IO.File]::ReadAllText('"+escaped+"'),$o); "
            "$d.RootElement.GetRawText() | ConvertFrom-Json -AsHashtable | ConvertTo-Json -Depth 20 -Compress")
        verify_user_settings(parsed)
        return {"path": str(path), "present": True, "bytes": value["bytes"], "sha256": value["sha256"]}

    def removal(name):
        inventory = registry_inventory(); normal = normal_path()
        facts = {"installed_inventory_preserved": inventory == before_inventory,
            "path_preserved": base.path_hashes() == before_path, "state_preserved": candidate.tree(fixture) == before_state,
            "links_preserved": link_inventory(links) == before_links,
            "package_directories_preserved": link_inventory(packages) == before_package_dirs,
            "package_directory_exists": installed_root is not None and os.path.lexists(installed_root),
            "aliases": {alias: {"link_exists": os.path.lexists(links/alias), "resolution": shutil.which(alias, path=normal)}
                        for alias in ("flere.exe", "flere-connect.exe")}}
        run.record(name, facts); run.record(name+"-inventory", inventory)
        verify_removal(facts)
        return facts

    try:
        gh = shutil.which("gh.exe"); require(gh, "existing GitHub CLI missing")
        def load_input(chosen, directory, prefix):
            api = f"repos/{base.REPO}/actions/artifacts/{chosen['artifact']}"
            value = json.loads(run.command(prefix+"artifact-api", [gh,"api",api], env=os.environ.copy(), maximum=65536))
            base.verify_api(value, chosen); run.record(prefix+"artifact-api", value)
            artifact = run.command(prefix+"artifact-download", [gh,"api",api+"/zip"], env=os.environ.copy(),
                                   destination=directory/"artifact.zip", maximum=chosen["artifact_bytes"])
            portable, _, _, manifest, payload, original = base.inputs(artifact, directory, chosen)
            if chosen["owner_check"]:
                require(sha(payload["manifest.json"]) == chosen["manifest_sha"], "candidate manifest bytes differ")
            run.record(prefix+"input-recipe-receipt", original); run.record(prefix+"manifest", manifest)
            return chosen, artifact, portable, manifest, payload
        target = load_input(selected, work, "")
        phases = [("", target)]
        if baseline is not None:
            old_work = work/"public034"; old_work.mkdir()
            phases.insert(0, ("baseline-", load_input(baseline, old_work, "baseline-")))
        run.pwsh = Path(shutil.which("pwsh.exe") or "missing"); require(run.pwsh.is_file(), "native PowerShell7 unavailable")
        info = run.ps("machine", "$i=[Security.Principal.WindowsIdentity]::GetCurrent(); $p=[Security.Principal.WindowsPrincipal]$i; "
            "[pscustomobject]@{edition=$PSVersionTable.PSEdition;major=$PSVersionTable.PSVersion.Major;home=$PSHOME;"
            "elevated=$p.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator);"
            "caption=(Get-CimInstance Win32_OperatingSystem).Caption;locale=[Globalization.CultureInfo]::CurrentUICulture.Name} | ConvertTo-Json -Compress")
        require(info["edition"] == "Core" and info["major"] == 7 and info["elevated"] is True
                and info["locale"] == "en-US" and "Windows Server 2025" in info["caption"]
                and (Path(info["home"])/"pwsh.exe").resolve() == run.pwsh.resolve(),
                "already elevated hosted Windows2025/native PowerShell7 required")
        run.record("machine", info)
        located = shutil.which("winget.exe"); require(located, "preinstalled WinGet unavailable")
        winget = Path(located); local_appdata = Path(os.environ["LOCALAPPDATA"])
        require(winget == local_appdata/"Microsoft/WindowsApps/winget.exe", "normal preinstalled WinGet app alias required")
        identity = winget.lstat()
        run.record("manager", {"path": str(winget), "file_id": identity.st_ino, "device": identity.st_dev,
                   "reparse_tag": getattr(identity, "st_reparse_tag", None), "version_expected": WINGET_VERSION})
        require(run.command("winget-version", [winget,"--version"]).decode("utf-8-sig").strip() == WINGET_VERSION, "reviewed WinGet version differs")
        run.command("winget-info", [winget,"--info"]); receipt["machine"]["winget"] = WINGET_VERSION
        links = local_appdata/"Microsoft/WinGet/Links"
        packages = local_appdata/"Microsoft/WinGet/Packages"
        settings_before = settings("settings-before"); user_before = user_file("user-settings-before")
        source_record = "sources-provision-before" if upgrade else "sources-before"
        observed_sources = source_export(run.command(source_record, [winget,"source","export"]))
        run.record(source_record, observed_sources)
        if upgrade:
            expected_sources = upgrade_source_baseline(observed_sources)
            receipt["source_provisioning"] = {"status":"attempted", "removed_name":"msstore",
                "retained_until_vm_teardown":True}
            receipt["limits"].append("Upgrade-only disposable VM provisioning removes the observed default Store source; the community-only source baseline is preserved until VM teardown, not restored to stock sources.")
            run.command("provision-remove-store", [winget,"source","remove","--name","msstore"])
            observed_sources = source_export(run.command("sources-provision-after", [winget,"source","export"]))
            run.record("sources-provision-after", observed_sources)
            require(observed_sources == expected_sources, "source provisioning changed beyond removal of the observed Store source")
            receipt["source_provisioning"]["status"] = "complete"
            run.record("sources-before", observed_sources)
        sources_before = observed_sources
        # No source reset, account, automatic agreements or custom source. Normal metadata access is explicit.
        run.command("community-source-update", [winget,"source","update","--name","winget"], seconds=180, source_prompt=True)
        run.command("installed-list-before", [winget,"list","--source","winget"], seconds=180, source_prompt=True)
        before_inventory = registry_inventory(); require(not any(row["values"] for row in before_inventory), "package already installed")
        run.record("installed-before", before_inventory)
        before_path = base.path_hashes(); before_links = link_inventory(links)
        before_normal_path = normal_path()
        verify_link_path_baseline(before_normal_path, links, before_links)
        require(all(shutil.which(alias, path=before_normal_path) is None and not os.path.lexists(links/alias)
                    for alias in ("flere.exe", "flere-connect.exe")), "alias collision")
        before_package_dirs = link_inventory(packages)
        require(not any("flere" in name.lower() for name in before_package_dirs), "existing portable Flere directory")
        run.record("path-before", before_path); run.record("links-before", before_links)
        fixture = work/"profile"; fixture.mkdir()
        for name in ("state", ".ssh", "AppData/Local", "AppData/Roaming", ".cache", "temp"):
            (fixture/name).mkdir(parents=True, exist_ok=True)
        (fixture/"state/preserved.json").write_text('{"synthetic":true,"selection":"retained"}\n')
        (fixture/".ssh/config").write_text("# synthetic sentinel; no SSH is launched\n")
        before_state = candidate.tree(fixture)
        child_env = dict(run.env, PATH=before_normal_path, HOME=str(fixture), USERPROFILE=str(fixture),
            LOCALAPPDATA=str(fixture/"AppData/Local"), APPDATA=str(fixture/"AppData/Roaming"),
            XDG_CACHE_HOME=str(fixture/".cache"), TEMP=str(fixture/"temp"), TMP=str(fixture/"temp"))
        if selected["owner_check"]:
            child_env.update(XDG_CONFIG_HOME=str(fixture/"AppData/Roaming"),
                XDG_DATA_HOME=str(fixture/"AppData/Local"), XDG_STATE_HOME=str(fixture/"state"))
            # Explicitly initialize only the product's normal PowerShell dependency
            # before install or product execution, then preserve that recorded baseline.
            cold_state = before_state
            shell = shutil.which("powershell.exe", path=child_env["PATH"])
            require(shell is not None, "normal Windows PowerShell dependency unavailable")
            require(run.command("initialize-powershell", [shell,"-NoProfile","-NonInteractive","-Command",
                    "[System.Console]::Out.Write('flere-tool-baseline')"], env=child_env,
                    seconds=30, maximum=4096) == b"flere-tool-baseline", "PowerShell initialization output differs")
            before_state = candidate.tree(fixture); after_paths = base.path_hashes()
            run.record("tool-initialization-state", base.preservation_snapshot(cold_state, before_state, before_path, after_paths))
            verify_powershell_initialization(cold_state, before_state)
            require(after_paths == before_path, "tool initialization changed PATH")
            receipt["limits"].append("Synthetic profile preservation starts after recorded normal PowerShell initialization; cold-profile no-write behavior is not claimed.")
        run.record("state-before", before_state)
        if not setting_value(settings_before):
            changed_setting = True
            run.command("enable-local-manifest", [winget,"settings","--enable","LocalManifestFiles"])
        verify_settings(settings_before, settings("settings-during"), True)
        baseline_record = None
        for prefix, (phase_input, artifact, portable, manifest, payload) in phases:
            version = base.input_version(phase_input)
            manifests = work/(prefix+"manifests")
            mirror = base.Mirror(portable, phase_input); mirrors.append((prefix, mirror, phase_input))
            run.record(prefix+"private-manifests", local_manifests(artifact, manifests, mirror.server_port, phase_input))
            run.command(prefix+"validate", [winget,"validate",manifests])
            attempted = True
            action = "upgrade" if upgrade and not prefix else "install"
            arguments = [winget,action,"--manifest",manifests,"--scope","user","--architecture","x64"]
            run.command(prefix+action, arguments, seconds=180)
            after_inventory = registry_inventory(); run.record(prefix+"installed-after", after_inventory)
            verify_inventory(before_inventory, after_inventory, True, version=version); owned = own_record(after_inventory, version=version)
            if upgrade:
                if prefix:
                    baseline_record = owned
                else:
                    verify_upgrade_identity(baseline_record, owned)
                    receipt["upgrade_identity_verified"] = True
            installed_root = Path(owned["values"]["InstallLocation"])
            require(installed_root.is_absolute() and installed_root.parent.resolve() == packages.resolve(), "portable path outside default user root")
            base.file_record(packages); base.file_record(installed_root)
            entries = list(installed_root.iterdir()); extras = [p for p in entries if p.name not in payload]
            require(len(entries) == len(payload)+1 and len(extras) == 1 and extras[0].suffix == ".db", "portable payload/index inventory differs")
            installed_files = [base.file_record(path) for path in entries]
            for name, data in payload.items():
                require(base.file_record(installed_root/name)["sha256"] == sha(data), "installed payload differs")
            require(extras[0].stat().st_size <= 512*1024, "portable index exceeds bound")
            (proof/"records"/(prefix+"portable-index.bin")).write_bytes(extras[0].read_bytes())
            run.record(prefix+"installed-layout", {"registry": owned, "files": installed_files,
                       "index_retained": prefix+"portable-index.bin", "identity_note": "Observed native file IDs/hashes; any owner-query results are recorded separately."})
            listing = run.command(prefix+"installed-list-exact", [winget, *local_package_args(owned, version=version)], source_prompt=True)
            require(PACKAGE.encode() in clean_console(listing) and version.encode() in listing, "normal installed manager identity missing")
            after_links = link_inventory(links)
            require({k:v for k,v in after_links.items() if k not in ("flere.exe","flere-connect.exe")} == before_links, "unrelated portable links changed")
            child_env["PATH"] = normal_path()
            aliases = {}
            for alias in ("flere.exe", "flere-connect.exe"):
                path = Path(shutil.which(alias, path=child_env["PATH"]) or "missing")
                require(path == links/alias and path.is_symlink() and path.resolve() == (installed_root/alias).resolve(), "normal WinGet PATH symlink differs")
                aliases[alias] = {"link": after_links[alias], "resolved": base.file_record(path.resolve())}
                for flag in ("--help", "--version", "--build-info"):
                    data = run.command(prefix+alias[:-4]+"-"+flag[2:], [path,flag], env=child_env, seconds=20, maximum=32768)
                    if flag == "--build-info":
                        require(json.loads(data) == manifest["build"], "alias full build identity differs")
                    elif flag == "--version":
                        require(data.decode().strip().startswith("flere-connect "+version+" ") and manifest["build"]["build_id"] in data.decode(), "alias version differs")
                    else:
                        require(b"--build-info" in data and b"ssh" in data, "alias help differs")
                require(base.file_record(path.resolve()) == aliases[alias]["resolved"], "alias payload changed")
            run.record(prefix+"aliases", aliases); require(candidate.tree(fixture) == before_state, "stateless calls changed state")
            if not prefix and phase_input["owner_check"]:
                owner_paths = base.path_hashes(); owner_statuses = {}
                run.record(prefix+"owner-synthetic-environment", verify_synthetic_override(child_env, installed_root/"flere.exe"))
                for alias in ("flere.exe", "flere-connect.exe"):
                    executable = (installed_root/alias).resolve()
                    data = run.command(prefix+alias[:-4]+"-update-status", [links/alias,"update-status"],
                                       env=child_env, seconds=60, maximum=32768)
                    value = json.loads(data)
                    owner_statuses[alias] = verify_installed_owner(value, manifest, executable)
                    run.record(prefix+alias[:-4]+"-update-status", value)
                run.record(prefix+"installed-ownership", owner_statuses)
                owner_after_state = candidate.tree(fixture)
                owner_after_paths = base.path_hashes()
                run.record(prefix+"owner-state-preservation", base.preservation_snapshot(before_state, owner_after_state, owner_paths, owner_after_paths))
                require(owner_after_state == before_state and owner_after_paths == owner_paths,
                        "owner queries changed the initialized profile or PATH")
                owner_inventory = registry_inventory(); owner_links = link_inventory(links)
                owner_files = [base.file_record(path) for path in entries]
                run.record(prefix+"owner-installed-after", {"registry":owner_inventory,"links":owner_links,"files":owner_files})
                require(owner_inventory == after_inventory and owner_links == after_links
                        and set(installed_root.iterdir()) == set(entries) and owner_files == installed_files,
                        "owner queries changed installed records, links or payload/index identity")
                receipt["installed_owner_aliases_checked"] = 2
                receipt["installed_owner_guidance_verified"] = True
        run.command("uninstall", [winget, *local_package_args(owned, remove=True, version=target_version)], seconds=180, source_prompt=True)
        removed = True; removal("removal")
        run.command("installed-list-after", [winget,"list","--source","winget"], source_prompt=True)
        require(link_inventory(packages) == before_package_dirs, "unrelated portable package directories changed")
        receipt.update(status="lifecycle_complete_pending_cleanup", aliases_checked=(12 if upgrade else 6),
            installed_inventory_preserved=True, state_preserved=True, path_preserved=True, no_product_owner_claim=not selected["owner_check"])
    except BaseException as error:
        receipt.update(status="failed", error=str(error), traceback=traceback.format_exc())
    finally:
        run.deadline = time.monotonic()+180; errors = []
        if attempted and not removed:
            try:
                current = registry_inventory()
                if any(row["values"] for row in current):
                    rows = [row for row in current if row["values"].get("WinGetPackageIdentifier") == PACKAGE]
                    require(len(rows) == 1, "exact cleanup package record required")
                    cleanup_version = rows[0]["values"].get("DisplayVersion")
                    require(cleanup_version in ({"0.3.4", "0.3.5"} if upgrade else {target_version}), "unexpected cleanup package version")
                    owned = own_record(current, complete=False, version=cleanup_version)
                    # RegisterARPEntry precedes directory creation; a partial
                    # owned record still receives ordinary manager cleanup.
                    if owned["values"].get("InstallLocation"):
                        path = Path(owned["values"]["InstallLocation"])
                        require(path.is_absolute() and path.parent.resolve() == packages.resolve(), "cleanup record path differs")
                        installed_root = path
                    run.command("failure-uninstall", [winget, *local_package_args(owned, remove=True, version=cleanup_version)], source_prompt=True)
                removal("failure-removal")
            except BaseException as error:
                errors.append("normal uninstall: "+str(error))
        if changed_setting:
            try:
                run.command("restore-local-manifest", [winget,"settings","--disable","LocalManifestFiles"])
            except BaseException as error:
                errors.append("settings restore: "+str(error))
        if settings_before is not None:
            try:
                verify_settings(settings_before, settings("settings-restored"), setting_value(settings_before))
                if user_before is not None:
                    require(user_file("user-settings-restored") == user_before, "user settings changed")
                if sources_before is not None:
                    require(source_export(run.command("sources-restored", [winget,"source","export"])) == sources_before, "source definitions changed")
                    receipt["sources_preserved"] = True
                receipt["settings_restored"] = True
            except BaseException as error:
                errors.append("settings/source verification: "+str(error))
        for prefix, mirror, mirror_input in mirrors:
            try:
                mirror.close_owned(); receipt[prefix+"loopback_closed"] = True
                base.verify_mirror(mirror.requests, mirror.attempts, mirror.rejections, mirror.rejections_dropped, mirror.internal_errors, mirror_input)
                receipt[prefix+"loopback_verified"] = True
            except BaseException as error:
                errors.append(prefix+"loopback: "+str(error))
            for key, value in {"loopback_requests":mirror.requests, "loopback_response_attempts":mirror.attempts,
                "loopback_rejections":mirror.rejections, "loopback_rejections_dropped":mirror.rejections_dropped,
                "loopback_internal_errors":mirror.internal_errors}.items():
                receipt[prefix+key] = value
        receipt["cleanup_errors"] = errors
        if errors:
            receipt["status"] = "failed"
        elif receipt["status"] == "lifecycle_complete_pending_cleanup":
            receipt["status"] = "lifecycle_passed"
        files = [path for path in proof.rglob("*") if path.is_file() and path.name != "receipt.json"]
        if len(files) > (128 if upgrade else 96) or sum(path.stat().st_size for path in files) > 8*1024*1024:
            receipt.update(status="failed", error="curated proof bound exceeded")
        receipt["files"] = {path.relative_to(proof).as_posix(): {"bytes":path.stat().st_size,"sha256":sha(path.read_bytes())} for path in sorted(files)}
        run.save()
    require(receipt["status"] == "lifecycle_passed", "WinGet lifecycle failed; inspect bounded proof")



def self_test():
    """Pure protocol/preservation regressions; no manager, native payload or network."""
    import unittest
    class Guards(unittest.TestCase):
        def source(self):
            return {"Name":"winget", "Arg":SOURCE_URL, "Type":"Microsoft.PreIndexed.Package",
                    "Identifier":SOURCE_ID, "Explicit":False, "Data":SOURCE_ID, "TrustLevel":["Trusted","StoreOrigin"]}

        def settings(self):
            return {"$schema":SETTING_SCHEMA,"userSettingsFile":"C:\\fixture\\settings.json",
                    "adminSettings":{"LocalManifestFiles":False,"InstallerHashOverride":False}}

        def owned(self):
            return {"key":"HKCU/64/observed","hive":"HKCU","view":"64","sha256":"a"*64,
                    "values":{"WinGetPackageIdentifier":PACKAGE,"DisplayVersion":VERSION,"DisplayName":"Flere Connect",
                              "WinGetInstallerType":"portable","WinGetSourceIdentifier":"*DefaultSource",
                              "InstallDirectoryCreated":1}}

        def test_hosted_guard_each_identity(self):
            env={"GITHUB_ACTIONS":"true","FLERE_RUNNER_ENVIRONMENT":"github-hosted","GITHUB_EVENT_NAME":"workflow_dispatch",
                 "GITHUB_REPOSITORY":base.REPO,"GITHUB_REF":"refs/heads/main","RUNNER_OS":"Windows","RUNNER_ARCH":"X64",
                 "FLERE_WORKFLOW_SHA":"a"*40,"GITHUB_RUN_ID":"1","GITHUB_RUN_ATTEMPT":"1"}
            candidate.hosted(env)
            for key in env:
                with self.subTest(key=key), self.assertRaises(ValueError):
                    candidate.hosted(dict(env, **{key:"unexpected"}))

        def test_source_prompt_every_split_and_one_answer(self):
            raw=SOURCE_TITLE+b"\r\n"+SOURCE_PROMPT+b"\r\n"+OPTIONS
            for at in range(len(raw)):
                self.assertFalse(prompt_ready(raw[:at],False,True))
                self.assertTrue(prompt_ready(raw[:at]+raw[at:],False,True))
            self.assertFalse(prompt_ready(raw,True,True))
            self.assertFalse(prompt_ready(b"Found Flere Connect. Installing...",False,False))

        def test_changed_unknown_repeated_and_out_of_phase_prompts(self):
            good=SOURCE_TITLE+b"\n"+SOURCE_PROMPT+b"\n"+OPTIONS
            for raw,allowed in ((good,False),(OPTIONS,True),(good+OPTIONS,True),(good.replace(b"`winget`",b"`msstore`"),True),
                                (good.replace(SOURCE_PROMPT,b"2-letter geographic region\n"+SOURCE_PROMPT),True),
                                (b"The publisher requires that you view terms.\n"+OPTIONS,True)):
                with self.subTest(raw=raw), self.assertRaises(ValueError): prompt_ready(raw,False,allowed)

        def test_actual_source_export_object_stream(self):
            other=dict(self.source(),Name="msstore",Arg="https://storeedgefd.dsx.mp.microsoft.com/v9.0")
            raw=(json.dumps(other)+"\n"+json.dumps(self.source())+"\n").encode()
            self.assertEqual(len(source_export(raw)),2)
            for bad in (raw+b"warning",json.dumps([self.source()]).encode(),raw+json.dumps(self.source()).encode(),
                        json.dumps(dict(self.source(),Arg="http://127.0.0.1")).encode()):
                with self.assertRaises((ValueError,json.JSONDecodeError)):source_export(bad)

        def test_upgrade_source_provisioning_preserves_exact_community_record(self):
            store={"Name":"msstore","Arg":"https://storeedgefd.dsx.mp.microsoft.com/v9.0",
                   "Data":"","Explicit":False,"Identifier":"StoreEdgeFD",
                   "TrustLevel":["Trusted"],"Type":"Microsoft.Rest"}
            original=[store,self.source()]
            before=copy.deepcopy(original)
            expected=upgrade_source_baseline(original)
            self.assertEqual(expected,[self.source()])
            self.assertEqual(original,before)
            for changed in ([store], [self.source()], original+[self.source()],
                            [dict(store,Arg="https://other.invalid"),self.source()],
                            [store,dict(self.source(),TrustLevel=["Trusted"])],
                            [dict(store,Name="unrelated"),self.source()]):
                with self.subTest(changed=changed), self.assertRaises(ValueError):
                    upgrade_source_baseline(changed)

        def test_observed_boolean_setting_only_and_restoration(self):
            old=self.settings();new=copy.deepcopy(old);new["adminSettings"]["LocalManifestFiles"]=True
            verify_settings(old,new,True);verify_settings(old,old,False);verify_settings(new,new,True)
            for key,val in (("InstallerHashOverride",True),("LocalManifestFiles","false")):
                bad=copy.deepcopy(old);bad["adminSettings"][key]=val
                with self.assertRaises(ValueError):verify_settings(old,bad,False)
            bad=copy.deepcopy(new);bad["userSettingsFile"]="C:\\other\\settings.json"
            with self.assertRaises(ValueError):verify_settings(old,bad,True)

        def test_custom_root_prompt_and_purge_settings_rejected(self):
            verify_user_settings({"visual":{"progressBar":"accent"}})
            for value in ({"installBehavior":{"portablePackageUserRoot":"C:\\other"}},
                          {"interactivity":{"disable":True}},{"uninstallBehavior":{"purgePortablePackage":True}},
                          {"interactivity":{"disable":"false"}}):
                with self.assertRaises(ValueError):verify_user_settings(value)

        def test_existing_empty_links_path_rejected_with_windows_spelling(self):
            root=r"C:\Users\fixture\AppData\Local\Microsoft\WinGet\Links"
            for value in (root, root.upper()+"\\", '"'+root+'\\"', root.replace("\\","/")):
                with self.assertRaises(ValueError):verify_link_path_baseline("C:\\bin;"+value,root,{})
            verify_link_path_baseline("C:\\bin",root,{})
            verify_link_path_baseline(root,root,{"unrelated.exe":{"link":"unchanged"}})

        def test_active_owner_record_and_wrong_identity(self):
            own=self.owned();self.assertEqual(own_record([own]),own)
            for key,val in (("WinGetSourceIdentifier","elsewhere"),("DisplayVersion","9.0.0"),("WinGetInstallerType","exe")):
                bad=copy.deepcopy(own);bad["values"][key]=val
                with self.assertRaises(ValueError):own_record([bad])
            with self.assertRaises(ValueError):own_record([own,own])

        def test_partial_owned_record_can_receive_normal_cleanup(self):
            own=self.owned();del own["values"]["InstallDirectoryCreated"]
            with self.assertRaises(ValueError):own_record([own])
            self.assertEqual(own_record([own],complete=False),own)
            own["values"]["InstallDirectoryAddedToPath"]=1
            self.assertEqual(own_record([own],complete=False),own)
            with self.assertRaises(ValueError):own_record([own])

        def test_local_manifest_manager_selection_uses_observed_record_not_catalog_id(self):
            # WinGet 1.11.510 retained this shape after a normal local-manifest
            # install; the generated ARP uninstall command uses --product-code.
            owned = {"hive": "HKCU", "view": "64", "subkey": ARP + "\\" + PRODUCT_CODE,
                "values": {"WinGetPackageIdentifier": PACKAGE, "DisplayName": "Flere Connect",
                    "DisplayVersion": VERSION, "WinGetInstallerType": "portable",
                    "WinGetSourceIdentifier": "*DefaultSource",
                    "UninstallString": "winget uninstall --product-code " + PRODUCT_CODE}}
            listing = local_package_args(owned)
            removal = local_package_args(owned, remove=True)
            self.assertEqual(listing, ["list", "--name", "Flere Connect", "--exact", "--scope", "user", "--source", "winget"])
            self.assertEqual(removal, ["uninstall", "--product-code", PRODUCT_CODE, "--exact", "--scope", "user", "--source", "winget"])
            self.assertNotIn("--manifest", removal)
            self.assertNotIn("--id", listing)

        def test_local_manager_selection_rejects_other_or_injected_arp_target(self):
            owned = {"hive": "HKCU", "view": "64", "subkey": ARP + "\\" + PRODUCT_CODE,
                "values": {"WinGetPackageIdentifier": PACKAGE, "DisplayName": "Flere Connect",
                    "DisplayVersion": VERSION, "WinGetInstallerType": "portable",
                    "WinGetSourceIdentifier": "*DefaultSource",
                    "UninstallString": "winget uninstall --product-code " + PRODUCT_CODE}}
            for field, replacement in (("subkey", ARP + "\\OtherPackage"), ("hive", "HKLM"),
                ("WinGetSourceIdentifier", SOURCE_ID), ("DisplayVersion", "0.0.0"),
                ("UninstallString", "winget uninstall --product-code OtherPackage"),
                ("UninstallString", "winget uninstall --product-code " + PRODUCT_CODE + " & extra")):
                changed = copy.deepcopy(owned)
                if field in changed: changed[field] = replacement
                else: changed["values"][field] = replacement
                for remove in (False, True):
                    with self.subTest(field=field, remove=remove), self.assertRaises(ValueError):
                        local_package_args(changed, remove=remove)

        def test_unrelated_inventory_and_own_key_preservation(self):
            old=[{"key":"HKLM/64/other","sha256":"b"*64,"values":{}}];own=self.owned()
            verify_inventory(old,old+[own],True);verify_inventory(old,old,False)
            for after,installed in (([dict(old[0],sha256="c"*64),own],True),(old+[own],False),(old,True)):
                with self.assertRaises(ValueError):verify_inventory(old,after,installed)
            with self.assertRaises(ValueError):verify_inventory(old+[own],old+[own],True)

        def test_removal_rejects_each_residual_or_preservation_failure(self):
            value={key:True for key in ("installed_inventory_preserved","path_preserved","state_preserved",
                                       "links_preserved","package_directories_preserved")}
            value.update(package_directory_exists=False,aliases={name:{"link_exists":False,"resolution":None}
                         for name in ("flere.exe","flere-connect.exe")})
            verify_removal(value)
            for key in ("installed_inventory_preserved","path_preserved","state_preserved","links_preserved","package_directories_preserved"):
                with self.assertRaises(ValueError):verify_removal(dict(value,**{key:False}))
            with self.assertRaises(ValueError):verify_removal(dict(value,package_directory_exists=True))
            changed=copy.deepcopy(value);changed["aliases"]["flere.exe"]["resolution"]="C:\\unexpected\\flere.exe"
            with self.assertRaises(ValueError):verify_removal(changed)
            changed=copy.deepcopy(value);del changed["aliases"]["flere-connect.exe"]
            with self.assertRaises(ValueError):verify_removal(changed)

        def test_actual_frozen_manifest_single_url_substitution(self):
            original=base64.b64decode("IyB5YW1sLWxhbmd1YWdlLXNlcnZlcjogJHNjaGVtYT1odHRwczovL2FrYS5tcy93aW5nZXQtbWFuaWZlc3QuaW5zdGFsbGVyLjEuMTAuMC5zY2hlbWEuanNvbg0KUGFja2FnZUlkZW50aWZpZXI6IFJvYmVydENyb25pbi5GbGVyZUNvbm5lY3QNClBhY2thZ2VWZXJzaW9uOiAiMC4zLjQiDQpJbnN0YWxsZXJUeXBlOiB6aXANCk5lc3RlZEluc3RhbGxlclR5cGU6IHBvcnRhYmxlDQpJbnN0YWxsZXJzOg0KLSBBcmNoaXRlY3R1cmU6IHg2NA0KICBOZXN0ZWRJbnN0YWxsZXJGaWxlczoNCiAgLSBSZWxhdGl2ZUZpbGVQYXRoOiBmbGVyZS5leGUNCiAgICBQb3J0YWJsZUNvbW1hbmRBbGlhczogZmxlcmUNCiAgLSBSZWxhdGl2ZUZpbGVQYXRoOiBmbGVyZS1jb25uZWN0LmV4ZQ0KICAgIFBvcnRhYmxlQ29tbWFuZEFsaWFzOiBmbGVyZS1jb25uZWN0DQogIEluc3RhbGxlclVybDogImh0dHBzOi8vZ2l0aHViLmNvbS9yb2JlcnQtY3JvbmluL2ZsZXJlL3JlbGVhc2VzL2Rvd25sb2FkL3YwLjMuNC9mbGVyZS1jb25uZWN0LTAuMy40LXg4Nl82NC1wYy13aW5kb3dzLW1zdmMuemlwIg0KICBJbnN0YWxsZXJTaGEyNTY6IEU1NjJGOTkxNjhBMzQyMDJFQ0FBMkRDMTYzREIyNjVFOURGM0MwMDA3Njk5RkJCN0ZFQTZCMjRDNkM4RTczOTINCk1hbmlmZXN0VHlwZTogaW5zdGFsbGVyDQpNYW5pZmVzdFZlcnNpb246IDEuMTAuMA0K")
            name=PACKAGE+".installer.yaml";port=49152
            changed=local_manifest(name,original,port)
            local=f"http://127.0.0.1:{port}/{base.ZIP_NAME}".encode()
            self.assertEqual(changed.replace(local,base.PUBLIC_URL),original)
            self.assertIn(base.ZIP_SHA.upper().encode(),changed)
            for data in (original+b"\n",original.replace(base.ZIP_SHA.upper().encode(),b"A"*64)):
                with self.assertRaises(ValueError):local_manifest(name,data,port)
            for bad in (True,80,65536,"49152"):
                with self.assertRaises(ValueError):local_manifest(name,original,bad)

        def test_completed_object_mirror_contract_is_reused(self):
            row={"status":200,"path":"/"+base.ZIP_NAME,"method":"GET","content_length":1771884,
                 "bytes":1771884,"sha256":base.ZIP_SHA}
            base.verify_mirror([row],1,[{"status":400}],0,0)
            for rows,attempts,dropped,errors in (([dict(row,bytes=1)],1,0,0),([],1,0,0),([row],1,1,0),([row],1,0,1)):
                with self.assertRaises(ValueError):base.verify_mirror(rows,attempts,[],dropped,errors)

        def test_runtime_credentials_removed_without_module_override(self):
            self.assertEqual(base.runtime_env({"GH_TOKEN":"fake","PATH":"normal","PSModulePath":"normal-modules"}),
                             {"PATH":"normal","PSModulePath":"normal-modules"})

    class OwnerGuards(unittest.TestCase):
        def setUp(self):
            self.root = PureWindowsPath(r"C:\Users\fixture\AppData\Local\Microsoft\WinGet\Packages\RobertCronin.FlereConnect__DefaultSource")
            self.manifest = {"build": {"component": "flere-connect", "build_id": "exact", "package_version": "0.3.4"},
                             "payload": {"sha256": "a" * 64}}
            self.value = {"schema_version": 1, "running_build": self.manifest["build"], "installation": None,
                          "other_frontends": "untracked", "ownership": {"kind": "manager", "attempt": None,
                          "sha256": "a" * 64, "guidance": GUIDANCE}}

        def test_both_actual_alias_paths_and_extended_windows_spelling(self):
            for alias in ("flere.exe", "flere-connect.exe"):
                executable = self.root / alias
                self.value["ownership"]["executable"] = "\\\\?\\" + str(executable).upper()
                self.assertEqual(verify_installed_owner(self.value, self.manifest, executable), self.value["ownership"])
                other = self.root / ("flere.exe" if alias == "flere-connect.exe" else "flere-connect.exe")
                with self.assertRaises(ValueError):
                    verify_installed_owner(self.value, self.manifest, other)

        def test_wrong_owner_build_payload_extra_fields_and_guidance_are_rejected(self):
            executable = self.root / "flere-connect.exe"
            self.value["ownership"]["executable"] = str(executable)
            for field, wrong in (("kind", "unknown"), ("kind", "manual"), ("kind", "managed"), ("attempt", "unexpected"),
                                 ("sha256", "b" * 64), ("executable", r"C:\other\flere-connect.exe"),
                                 ("guidance", "winget upgrade --id RobertCronin.FlereConnect"), ("extra", True)):
                value = copy.deepcopy(self.value); value["ownership"][field] = wrong
                with self.subTest(field=field), self.assertRaises(ValueError):
                    verify_installed_owner(value, self.manifest, executable)
            for field, wrong in (("running_build", {"build_id": "stale"}), ("installation", {}),
                                 ("other_frontends", "tracked"), ("schema_version", 2), ("extra", True)):
                with self.subTest(field=field), self.assertRaises(ValueError):
                    verify_installed_owner(dict(self.value, **{field: wrong}), self.manifest, executable)

        def test_synthetic_override_is_distinct_and_all_state_paths_stay_in_fixture(self):
            home = r"C:\Users\fixture\.cache\flere\tmp\owned\profile"
            env = {"HOME": home, "USERPROFILE": home}
            for key, path in {"LOCALAPPDATA": "AppData/Local", "APPDATA": "AppData/Roaming", "XDG_CONFIG_HOME": "AppData/Roaming",
                              "XDG_DATA_HOME": "AppData/Local", "XDG_STATE_HOME": "state", "XDG_CACHE_HOME": ".cache",
                              "TEMP": "temp", "TMP": "temp"}.items():
                env[key] = str(PureWindowsPath(home) / path)
            executable = self.root / "flere.exe"
            self.assertEqual(verify_synthetic_override(env, executable), env)
            for key in env:
                with self.subTest(key=key), self.assertRaises(ValueError):
                    verify_synthetic_override(dict(env, **{key: r"C:\Users\fixture\AppData\Local"}), executable)

    class InputGuards(unittest.TestCase):
        def test_fixed_selection_api_and_wrong_candidate_identity(self):
            self.assertEqual(selection(), LEGACY_INPUT)
            self.assertFalse(selection()["owner_check"])
            selected = selection("candidate-9a9183c")
            self.assertTrue(selected["owner_check"])
            value = {"id":selected["artifact"],"name":selected["artifact_name"],"size_in_bytes":selected["artifact_bytes"],
                "expired":False,"digest":"sha256:"+selected["artifact_sha"],
                "workflow_run":{"id":selected["run"],"head_sha":selected["workflow"]}}
            base.verify_api(value, selected)
            for name in ("public-0.3.4", "candidate-21cf68c", "https://example.invalid/zip", ""):
                with self.assertRaises(ValueError): selection(name)
            with self.assertRaises(ValueError): base.verify_api(value, LEGACY_INPUT)
            for field, wrong in (("id", 1), ("digest", "sha256:"+"0"*64), ("expired",True),
                                 ("workflow_run", {"id":selected["run"], "head_sha":"0"*40})):
                with self.assertRaises(ValueError): base.verify_api(dict(value, **{field:wrong}), selected)
            selected["winget"].clear()
            self.assertEqual(len(selection("candidate-9a9183c")["winget"]),3)

        def test_recorded_powershell_startup_additions_only(self):
            before={"state/preserved.json":"a"*64,".ssh/config":"b"*64}
            after=dict(before)
            for folder in ("AppData/Local/Microsoft","AppData/Local/Microsoft/Windows","AppData/Local/Microsoft/Windows/PowerShell"):
                after[folder]="directory"
            profile="AppData/Local/Microsoft/Windows/PowerShell/StartupProfileData-NonInteractive"
            after[profile]="c"*64
            verify_powershell_initialization(before,before)
            verify_powershell_initialization(before,after)
            for name,wrong in (("state/preserved.json","d"*64),("temp/chocolatey","directory"),
                               (profile,"symlink"),("unexpected","c"*64)):
                with self.assertRaises(ValueError): verify_powershell_initialization(before,dict(after,**{name:wrong}))

    class UpgradeGuards(unittest.TestCase):
        def test_same_version_and_unverified_upgrade_are_rejected(self):
            for chosen in (LEGACY_INPUT, CURRENT_INPUT, PROFILE_INPUT):
                self.assertIsNone(upgrade_baseline(chosen, False))
                with self.assertRaises(ValueError): upgrade_baseline(chosen, True)
            chosen = dict(PROFILE_INPUT, version="0.3.5")
            self.assertEqual(upgrade_baseline(chosen, True), PUBLIC034_INPUT)
            with self.assertRaises(ValueError): upgrade_baseline(dict(chosen, owner_check=False), True)
            with self.assertRaises(ValueError): upgrade_baseline(chosen, "true")

        def test_exact_upgrade_record_changes_version_without_changing_identity(self):
            old = Guards().owned()
            old["subkey"] = ARP + "\\" + PRODUCT_CODE
            old["values"]["InstallLocation"] = str(PureWindowsPath(r"C:\Users\fixture\AppData\Local\Microsoft\WinGet\Packages") / PRODUCT_CODE)
            new = copy.deepcopy(old); new["values"]["DisplayVersion"] = "0.3.5"
            verify_upgrade_identity(old, new)
            unrelated = {"key":"other", "values":{}, "sha256":"b"*64}
            verify_inventory([unrelated], [unrelated, new], True, version="0.3.5")
            for field, wrong in (("DisplayVersion", "0.3.4"), ("InstallLocation", r"C:\Other"),
                                 ("WinGetPackageIdentifier", "Other"), ("WinGetSourceIdentifier", "Other")):
                changed = copy.deepcopy(new); changed["values"][field] = wrong
                with self.assertRaises(ValueError): verify_upgrade_identity(old, changed)
            with self.assertRaises(ValueError): verify_upgrade_identity(old, dict(new, key="other"))
            with self.assertRaises(ValueError): verify_inventory([unrelated], [dict(unrelated, sha256="c"*64),new], True, version="0.3.5")

        def test_versioned_manifest_substitution_preserves_all_non_url_bytes(self):
            for version in ("0.3.4", "0.3.5"):
                chosen = dict(PROFILE_INPUT, version=version)
                name = PACKAGE + ".installer.yaml"
                url = f"https://github.com/{base.REPO}/releases/download/v{version}/{base.zip_name(chosen)}".encode()
                original = b"PackageVersion: " + version.encode() + b"\nInstallerUrl: " + url + b"\nInstallerSha256: " + chosen["zip_sha"].encode() + b"\n"
                chosen["winget"] = {name:sha(original)}
                changed = local_manifest(name, original, 50000, chosen)
                local = f"http://127.0.0.1:50000/{base.zip_name(chosen)}".encode()
                self.assertEqual(changed.replace(local, url), original)
                with self.assertRaises(ValueError): local_manifest(name, original+b"x", 50000, chosen)

    suite=unittest.TestSuite(unittest.defaultTestLoader.loadTestsFromTestCase(case) for case in (Guards, OwnerGuards, InputGuards, UpgradeGuards))
    result=unittest.TextTestRunner(verbosity=2).run(suite)
    require(result.wasSuccessful(),"WinGet lifecycle guard checks failed")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", nargs="?", type=Path)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--upgrade-from-public034", action="store_true")
    parser.add_argument("--input", choices=tuple(INPUTS), default=DEFAULT_INPUT)
    args = parser.parse_args()
    if args.self_test:
        self_test()
    else:
        require(args.output is not None, "output is required")
        main(args.output, selection(args.input), upgrade=args.upgrade_from_public034)
