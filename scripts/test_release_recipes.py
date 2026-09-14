#!/usr/bin/env python3
"""Offline recipe artifact integrity checks; inert inputs, no build or publication."""
import importlib.util
import json
from pathlib import Path
import shutil
import sys
import unittest
from unittest import mock

import test_release_automation as fixtures

spec = importlib.util.spec_from_file_location("release_recipes", Path(__file__).with_name("release-recipes.py"))
recipes = importlib.util.module_from_spec(spec)
spec.loader.exec_module(recipes)
VERSION, COMMIT, RUN, WORKFLOW = fixtures.VERSION, fixtures.COMMIT, fixtures.RUN, fixtures.WORKFLOW


class ReleaseRecipes(unittest.TestCase):
    def setUp(self):
        fixtures.ReleaseAutomation.setUp(self)
        self.pin = fixtures.ReleaseAutomation.seal(self)
        self.output = self.directory.with_name(self.directory.name + "-recipes")
        self.addCleanup(shutil.rmtree, self.output, True)

    def prepare(self, **changes):
        args = dict(directory=self.directory, output=self.output, version=VERSION, commit=COMMIT,
                    run_id=RUN, workflow_sha=WORKFLOW, descriptor_sha256=self.pin)
        args.update(changes)
        return recipes.prepare(**args)

    def test_exact_bundle_binds_all_recipes_and_excludes_neighboring_files(self):
        neighbor = self.directory.with_name(self.directory.name + "-private.txt")
        neighbor.write_bytes(b"PRIVATE_NEIGHBOR_DO_NOT_PACKAGE")
        self.addCleanup(neighbor.unlink)
        pinned = {p: p.read_bytes() for p in [
            recipes.ROOT / "packaging/linux/release.json",
            *sorted((recipes.ROOT / "packaging/homebrew/Formula").glob("*.rb"))]}
        receipt = self.prepare()
        self.assertEqual(receipt["status"], "prepared_not_published")
        self.assertEqual((receipt["version"], receipt["commit"], receipt["run_id"], receipt["workflow_sha"]),
                         (VERSION, COMMIT, RUN, WORKFLOW))
        self.assertEqual(receipt["release_descriptor_sha256"], self.pin)
        self.assertEqual(receipt["source_sha256"], self.source_digest)
        files = {p.relative_to(self.output).as_posix(): p.read_bytes()
                 for p in self.output.rglob("*") if p.is_file()}
        self.assertEqual(set(files), recipes.ARTIFACT_FILES)
        self.assertEqual(len(files), 10)
        self.assertEqual(receipt["files"], {name: {"bytes": len(data), "sha256": recipes.release.sha(data)}
                                          for name, data in files.items() if name in recipes.RECIPE_FILES})
        checksums = dict(line.split("  ", 1)[::-1] for line in files["SHA256SUMS"].decode().splitlines())
        self.assertEqual(checksums, {name: recipes.release.sha(data) for name, data in files.items() if name != "SHA256SUMS"})
        self.assertNotIn(neighbor.read_bytes(), b"".join(files.values()))
        lock = json.loads(files["linux/release-lock.json"])
        self.assertEqual((lock["version"], lock["source_commit"], lock["source_sha256"]), (VERSION, COMMIT, self.source_digest))
        for component in ("flere", "flere-connect"):
            text = files[f"homebrew/Formula/{component}.rb"].decode()
            self.assertIn(f'version "{VERSION}"', text)
            self.assertIn(receipt["source_archive"]["sha256"], text)
        self.assertEqual({p: p.read_bytes() for p in pinned}, pinned)
        shutil.rmtree(self.output)
        self.prepare()
        self.assertEqual({p.relative_to(self.output).as_posix(): p.read_bytes()
                          for p in self.output.rglob("*") if p.is_file()}, files)

    def test_wrong_descriptor_or_selected_identity_fails_before_output(self):
        for changes in ({"descriptor_sha256": None}, {"descriptor_sha256": "0" * 64}, {"version": "0.4.1"},
                        {"commit": "c" * 40}, {"workflow_sha": "c" * 40}, {"run_id": "2345"}):
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                self.prepare(**changes)
            self.assertFalse(self.output.exists())

    def test_corrupt_source_payload_or_extra_input_fails_before_output(self):
        for name in (self.source_archive.name, f"flere-{recipes.release.TARGET}"):
            path = self.directory / name
            original = path.read_bytes()
            path.write_bytes(original + b"corrupt")
            with self.subTest(name=name), self.assertRaises(ValueError):
                self.prepare()
            path.write_bytes(original)
            self.assertFalse(self.output.exists())
        (self.directory / "private-file").write_text("do not include")
        with self.assertRaisesRegex(ValueError, "allowlist"):
            self.prepare()
        self.assertFalse(self.output.exists())

    def test_generator_extra_file_or_symlink_cannot_receive_a_success_receipt(self):
        original = recipes.packages.main
        for symlink in (False, True):
            def inject(args):
                original(args)
                path = self.output / "linux/unexpected"
                if symlink:
                    path.symlink_to(self.directory / "release.json")
                else:
                    path.write_text("unexpected generator output")
            with self.subTest(symlink=symlink), mock.patch.object(recipes.packages, "main", side_effect=inject), self.assertRaises(ValueError):
                self.prepare()
            self.assertFalse((self.output / "recipe-receipt.json").exists())
            shutil.rmtree(self.output)

    def test_input_change_during_generation_is_rejected_before_receipt(self):
        original = recipes.packages.main
        def change(args):
            original(args)
            path = self.directory / f"flere-{recipes.release.TARGET}"
            path.write_bytes(path.read_bytes() + b"changed after validation")
        with mock.patch.object(recipes.packages, "main", side_effect=change), self.assertRaises(ValueError):
            self.prepare()
        self.assertFalse((self.output / "recipe-receipt.json").exists())

    def test_existing_symlink_or_candidate_nested_output_is_rejected(self):
        self.output.symlink_to(self.directory, target_is_directory=True)
        with self.assertRaises(ValueError):
            self.prepare()
        self.output.unlink()
        self.output.mkdir()
        with self.assertRaisesRegex(ValueError, "already exists"):
            self.prepare()
        with self.assertRaisesRegex(ValueError, "separate private home-cache"):
            self.prepare(output=self.directory / "nested-output")
        self.assertFalse((self.directory / "nested-output").exists())

    def test_workflow_uploads_only_the_bundle_after_public_verification(self):
        workflow = (recipes.ROOT / ".github/workflows/release.yml").read_text()
        job = workflow.split("  prepare-recipes:\n", 1)[1].split("\n  publish-core:", 1)[0]
        self.assertIn("needs: [select, linux, verify-public]", job)
        self.assertNotIn("contents: write", job)
        self.assertNotIn("id-token:", job)
        self.assertNotIn("environment: release", job)
        prefix = "${{ steps.recipes.outputs.directory }}/"
        upload = {line.strip().removeprefix(prefix) for line in job.splitlines() if line.strip().startswith(prefix)}
        self.assertEqual(upload, recipes.ARTIFACT_FILES)
        self.assertIn("include-hidden-files: true", job)
        self.assertIn("needs: [select, verify-public]", workflow.split("  publish-core:\n", 1)[1])


if __name__ == "__main__":
    unittest.main()
