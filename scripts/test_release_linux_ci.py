#!/usr/bin/env python3
"""Offline CI failure diagnostics against the unchanged developer logging contract."""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("release_linux_ci", Path(__file__).with_name("release-linux-ci.py"))
ci = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ci)


@unittest.skipUnless(os.name == "posix", "native Linux CI diagnostics require POSIX descriptors and executable scripts")
class ReleaseLinuxDiagnostics(unittest.TestCase):
    def setUp(self):
        cache = Path.home() / ".cache/flere/tmp"
        cache.mkdir(mode=0o700, parents=True, exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(prefix="ci-diagnostics-", dir=cache)
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.base = self.root / "cache/flere/dev-updates"
        self.base.mkdir(mode=0o700, parents=True)
        self.environment = {"HOME": str(self.root), "XDG_CACHE_HOME": str(self.root / "cache"),
                            "PATH": "/usr/bin:/bin", "PYTHONDONTWRITEBYTECODE": "1"}

    def log(self, name="candidate-fixture1", contents=b"error: fixture failure\n"):
        directory = self.base / name
        directory.mkdir(mode=0o700)
        path = directory / "validation.log"
        path.write_bytes(contents)
        return path

    def test_actual_dev_failure_surfaces_only_new_bounded_redacted_tail(self):
        self.log("candidate-existing", b"OLD-PRIVATE-CANDIDATE\n")
        checkout = self.root / "checkout"
        (checkout / "scripts").mkdir(parents=True)
        shutil.copyfile(Path(__file__).with_name("dev"), checkout / "scripts/dev")
        binaries = self.root / "bin"
        binaries.mkdir()
        cargo = binaries / "cargo"
        cargo.write_text(f"#!{sys.executable}\n" + '''import os,sys
sys.stdout.write("OLD-LOG-HEADER\\n" + "historical noise\\n" * 10000)
print("error: fixture compilation failed")
print(os.environ["OPENAI_API_KEY"])
print("ghp_" + "FixtureOnlyValueNotARealCredential")
print("Authorization: Bearer fixture-bearer")
print("https://fixture-user:fixture-password@example.invalid/path")
print("::error::forged annotation")
print("\\x1b]52;c;fixture\\x07")
print("test result: FAILED. 1 failed")
sys.exit(23)
''')
        cargo.chmod(0o755)
        environment = dict(self.environment, PATH=str(binaries) + ":/usr/bin:/bin",
                           OPENAI_API_KEY="fixture-environment-value-to-hide")
        diagnostics = io.StringIO()
        with contextlib.redirect_stderr(diagnostics), self.assertRaises(subprocess.CalledProcessError) as failure:
            ci.package_candidate(checkout, environment)
        # scripts/dev reports the failing Cargo command, then exits 1 itself.
        self.assertEqual(failure.exception.returncode, 1)
        text = diagnostics.getvalue()
        self.assertIn("error: fixture compilation failed", text)
        self.assertIn("test result: FAILED. 1 failed", text)
        for forbidden in ("OLD-PRIVATE-CANDIDATE", "OLD-LOG-HEADER", "fixture-environment-value-to-hide",
                          "FixtureOnlyValueNotARealCredential", "fixture-bearer", "fixture-password", "\x1b", "\x07"):
            self.assertNotIn(forbidden, text)
        self.assertFalse(any(line.startswith("::") for line in text.splitlines()))
        self.assertLessEqual(len(text), 16500)
        self.assertEqual({p.relative_to(checkout).as_posix() for p in checkout.rglob("*") if p.is_file()},
                         {"scripts/dev"})

    def test_missing_or_ambiguous_logs_do_not_replace_the_original_failure(self):
        for names in ((), ("candidate-fixture1", "candidate-fixture2")):
            def failed(*args, **kwargs):
                for name in names:
                    self.log(name)
                raise subprocess.CalledProcessError(29, ["fixture-command"])
            with self.subTest(names=names), mock.patch.object(ci, "run", side_effect=failed):
                diagnostics = io.StringIO()
                with contextlib.redirect_stderr(diagnostics), self.assertRaises(subprocess.CalledProcessError) as failure:
                    ci.package_candidate(self.root, self.environment)
                self.assertEqual(failure.exception.returncode, 29)
                self.assertIn("unique safe log tail was unavailable", diagnostics.getvalue())

    def test_candidate_and_log_symlinks_are_never_followed(self):
        path = self.log()
        outside = self.root / "unrelated-log"
        outside.write_text("UNRELATED-PRIVATE-CONTENT")
        path.unlink()
        path.symlink_to(outside)
        with self.assertRaises(OSError):
            ci.validation_tail(self.base, set(), self.environment)
        path.unlink()
        path.parent.rmdir()
        path.parent.symlink_to(self.root, target_is_directory=True)
        with self.assertRaises(OSError):
            ci.validation_tail(self.base, set(), self.environment)

    def test_partial_first_line_and_output_truncation_cannot_escape_prefix(self):
        self.log(contents=b"never-expose-partial-credential-" * 3000 + b"\n" +
                 (b"::error::" + b"x" * 490 + b"\n") * 60)
        tail = ci.validation_tail(self.base, set(), self.environment)
        self.assertNotIn("partial-credential", tail)
        self.assertLessEqual(len(tail), 16384)
        self.assertTrue(all(line.startswith("validation | ") for line in tail.splitlines()))

    def test_success_does_not_read_or_emit_private_logs(self):
        self.log(contents=b"PRIVATE-SUCCESS-LOG")
        with mock.patch.object(ci, "run", return_value=json.dumps({"artifact": "fixture"})), \
                mock.patch.object(ci, "validation_tail") as tail:
            diagnostics = io.StringIO()
            with contextlib.redirect_stderr(diagnostics):
                self.assertEqual(ci.package_candidate(self.root, self.environment), {"artifact": "fixture"})
            tail.assert_not_called()
            self.assertEqual(diagnostics.getvalue(), "")


if __name__ == "__main__":
    unittest.main()
