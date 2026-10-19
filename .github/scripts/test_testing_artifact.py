"""Exercise the provenance CLI with temporary binaries and GitHub API fixtures."""

import copy
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("testing-artifact.py")
SOURCE = "a" * 40
MERGE = "b" * 40
BASE = "c" * 40
ORIGINAL = "d" * 40
REPOSITORY = "gluwa/creditcoin3"
BINARIES = (
    "archiver",
    "attestor",
    "attestor_zombienet",
    "checkpoint-builder",
    "creditcoin3-node",
    "proof-gen-api-server",
    "query-cli",
)


class TestingArtifactTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.directory = self.root / "binaries"
        self.directory.mkdir()
        for name in BINARIES:
            (self.directory / name).write_bytes(f"binary:{name}".encode())
        self.artifact = {
            "id": 101,
            "name": "binary-for-testing-ci",
            "expired": False,
            "digest": "sha256:" + "1" * 64,
            "workflow_run": {"id": 202, "repository_id": 303, "head_sha": SOURCE},
        }
        self.run = {
            "id": 202,
            "path": ".github/workflows/ci.yml",
            "head_sha": SOURCE,
            "run_attempt": 1,
            "status": "completed",
            "conclusion": "success",
            "repository": {"id": 303, "full_name": REPOSITORY},
        }
        self.commit = {"sha": SOURCE, "parents": [{"sha": BASE}]}
        self.expected_source = SOURCE
        self.current_repository = "gluwa/asc-message-relayer"
        self.current_run = 404

    def cli(self, command, args, success=True):
        result = subprocess.run(
            [sys.executable, str(SCRIPT), command, *args],
            text=True,
            capture_output=True,
            timeout=10,
            check=False,
        )
        if success:
            self.assertEqual(result.returncode, 0, result.stderr)
            return json.loads(result.stdout)
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn("provenance validation failed:", result.stderr)
        self.assertEqual(result.stdout, "")
        return result.stderr

    def create(
        self, *, cache_hit=False, source=SOURCE, attempt=1, changes=None, success=True
    ):
        values = {
            "directory": str(self.directory),
            "repository": REPOSITORY,
            "source-repository": REPOSITORY,
            "source-sha": source,
            "run-id": "202",
            "run-attempt": str(attempt),
            "cache-hit": str(cache_hit).lower(),
            "source-key": "native-bins-v2-Linux-X64-config-workflow-sources",
            "build-options": "--release --features=fast-runtime",
            "workspace-opt-level": "1",
            "runner-os": "Linux",
            "runner-arch": "X64",
        }
        values.update(changes or {})
        return self.cli(
            "create", [f"--{key}={value}" for key, value in values.items()], success
        )

    def metadata_args(self):
        for name, value in (
            ("artifact", self.artifact),
            ("run", self.run),
            ("commit", self.commit),
        ):
            (self.root / f"{name}.json").write_text(json.dumps(value), encoding="utf-8")
        return [
            "--artifact-metadata",
            str(self.root / "artifact.json"),
            "--run-metadata",
            str(self.root / "run.json"),
            "--source-commit-metadata",
            str(self.root / "commit.json"),
            "--expected-artifact-id",
            "101",
            "--expected-run-id",
            "202",
            "--expected-source-sha",
            self.expected_source,
            "--current-repository",
            self.current_repository,
            "--current-run-id",
            str(self.current_run),
        ]

    def verify(self, success=True):
        return self.cli(
            "verify",
            [*self.metadata_args(), "--directory", str(self.directory)],
            success,
        )

    def manifest(self):
        return json.loads(
            (self.directory / "native-build.json").read_text(encoding="utf-8")
        )

    def write_manifest(self, value):
        (self.directory / "native-build.json").write_text(
            json.dumps(value), encoding="utf-8"
        )

    def test_create_and_verify_all_seven_binary_hashes(self):
        created = self.create()
        self.assertEqual(set(created["build"]["binaries"]), set(BINARIES))
        for name in BINARIES:
            self.assertEqual(
                created["build"]["binaries"][name],
                hashlib.sha256(f"binary:{name}".encode()).hexdigest(),
            )
        metadata = self.cli("validate-metadata", self.metadata_args())
        self.assertEqual(metadata["artifact_id"], 101)
        self.assertEqual(metadata["digest"], self.artifact["digest"])
        self.assertEqual(self.verify()["producer"], created["producer"])

    def test_normal_pull_request_merge_checkout(self):
        self.expected_source = MERGE
        self.commit = {"sha": MERGE, "parents": [{"sha": BASE}, {"sha": SOURCE}]}
        self.create(source=MERGE)
        self.assertEqual(self.verify()["producer"]["source_sha"], MERGE)

    def test_merge_relation_rejects_unrelated_single_octopus_or_forged_commits(self):
        self.expected_source = MERGE
        self.create(source=MERGE)
        for commit in (
            {"sha": MERGE, "parents": [{"sha": BASE}, {"sha": ORIGINAL}]},
            {"sha": MERGE, "parents": [{"sha": SOURCE}]},
            {
                "sha": MERGE,
                "parents": [{"sha": SOURCE}, {"sha": BASE}, {"sha": ORIGINAL}],
            },
            {"sha": MERGE, "parents": [{"sha": SOURCE}, {"sha": SOURCE}]},
            {"sha": ORIGINAL, "parents": [{"sha": BASE}, {"sha": SOURCE}]},
        ):
            with self.subTest(commit=commit):
                self.commit = commit
                self.verify(success=False)

    def test_same_current_run_may_be_in_progress_but_external_or_failed_run_cannot(
        self,
    ):
        self.create()
        self.run.update(status="in_progress", conclusion=None)
        self.verify(success=False)
        self.current_run = 202
        self.verify(
            success=False
        )  # Same numeric ID in another repository is insufficient.
        self.current_repository = REPOSITORY
        self.verify()
        for status, conclusion in (
            ("completed", "failure"),
            ("queued", None),
            ("in_progress", "success"),
        ):
            with self.subTest(status=status, conclusion=conclusion):
                self.run.update(status=status, conclusion=conclusion)
                self.verify(success=False)

    def test_rerun_accepts_prior_attempt_but_rejects_future_or_invalid_attempt(
        self,
    ):
        self.create(attempt=1)
        self.run["run_attempt"] = 3
        self.assertEqual(self.verify()["producer"]["run_attempt"], 1)
        for attempt in (0, -1, 4, True, "1"):
            with self.subTest(attempt=attempt):
                manifest = self.manifest()
                manifest["producer"]["run_attempt"] = attempt
                self.write_manifest(manifest)
                self.verify(success=False)

    def test_cache_hit_retains_compile_origin_and_updates_upload_provenance(self):
        original = self.create(source=ORIGINAL)
        refreshed = self.create(cache_hit=True, source=SOURCE, attempt=2)
        self.assertEqual(refreshed["build"], original["build"])
        self.assertEqual(refreshed["build"]["source_sha"], ORIGINAL)
        self.assertEqual(refreshed["producer"]["source_sha"], SOURCE)
        self.assertEqual(refreshed["producer"]["run_attempt"], 2)
        self.run["run_attempt"] = 2
        self.verify()

    def test_generic_default_optimization_build_and_cache_hit_remain_supported(self):
        changes = {
            "workspace-opt-level": "",
            "build-options": "--release",
            "repository": "other/producer",
            "source-repository": "other/source",
        }
        original = self.create(source=ORIGINAL, changes=changes)
        refreshed = self.create(cache_hit=True, changes=changes)
        self.assertEqual(refreshed["build"], original["build"])
        self.assertEqual(refreshed["producer"]["source_sha"], SOURCE)
        self.verify(
            success=False
        )  # Generic artifacts are not accepted as CC3 testing builds.

    def test_cache_hit_rejects_missing_manifest_wrong_settings_and_tampering(
        self,
    ):
        self.create(cache_hit=True, success=False)
        self.create(source=ORIGINAL)
        before = (self.directory / "native-build.json").read_bytes()
        for changes in (
            {"source-key": "different"},
            {"build-options": "--release"},
            {"workspace-opt-level": "3"},
            {"runner-os": "macOS"},
            {"runner-arch": "ARM64"},
            {"source-repository": "other/repository"},
        ):
            with self.subTest(changes=changes):
                self.create(cache_hit=True, changes=changes, success=False)
                self.assertEqual(
                    (self.directory / "native-build.json").read_bytes(), before
                )
        (self.directory / BINARIES[0]).write_bytes(b"modified cached binary")
        self.create(cache_hit=True, success=False)
        self.assertEqual((self.directory / "native-build.json").read_bytes(), before)

    def test_binary_tampering_is_rejected(self):
        self.create()
        for name in BINARIES:
            with self.subTest(name=name):
                path = self.directory / name
                original = path.read_bytes()
                path.write_bytes(original + b"tampered")
                self.verify(success=False)
                path.write_bytes(original)

    def test_metadata_must_bind_artifact_run_head_repository_workflow_and_digest(self):
        self.create()
        cases = (
            ("artifact", ("id",), 999),
            ("artifact", ("name",), "other"),
            ("artifact", ("expired",), True),
            ("artifact", ("digest",), None),
            ("artifact", ("digest",), "sha256:" + "x" * 64),
            ("artifact", ("workflow_run", "id"), 999),
            ("artifact", ("workflow_run", "head_sha"), ORIGINAL),
            ("artifact", ("workflow_run", "repository_id"), 999),
            ("run", ("id",), 999),
            ("run", ("path",), ".github/workflows/other.yml"),
            ("run", ("repository", "full_name"), "other/repository"),
            ("run", ("run_attempt",), 0),
        )
        for document, keys, value in cases:
            with self.subTest(document=document, keys=keys):
                saved = copy.deepcopy(getattr(self, document))
                target = getattr(self, document)
                for key in keys[:-1]:
                    target = target[key]
                target[keys[-1]] = value
                self.verify(success=False)
                setattr(self, document, saved)

    def test_manifest_configuration_and_producer_identity_must_match(self):
        self.create()
        saved = self.manifest()
        cases = (
            ("build", "build_options", "--release"),
            ("build", "workspace_opt_level", "3"),
            ("build", "runner_os", "macOS"),
            ("build", "runner_arch", "ARM64"),
            ("build", "repository", "other/source"),
            ("build", "source_key", ""),
            ("producer", "repository", "other/producer"),
            ("producer", "source_sha", ORIGINAL),
            ("producer", "run_id", 999),
        )
        for section, key, value in cases:
            with self.subTest(section=section, key=key):
                manifest = copy.deepcopy(saved)
                manifest[section][key] = value
                self.write_manifest(manifest)
                self.verify(success=False)

    def test_only_canonical_binary_names_are_accepted(self):
        self.create()
        saved = self.manifest()
        for bad_name in (
            "../archiver",
            "./archiver",
            "ARCHIVER",
            "/tmp/archiver",
            "extra",
        ):
            with self.subTest(name=bad_name):
                manifest = copy.deepcopy(saved)
                manifest["build"]["binaries"][bad_name] = manifest["build"][
                    "binaries"
                ].pop("archiver")
                self.write_manifest(manifest)
                self.verify(success=False)

    def test_binary_symlinks_directories_and_fifos_are_rejected_without_hanging(self):
        self.create()
        path = self.directory / BINARIES[0]
        path.unlink()
        for kind in ("symlink", "directory", "fifo"):
            with self.subTest(kind=kind):
                if kind == "symlink":
                    path.symlink_to(self.directory / BINARIES[1])
                elif kind == "directory":
                    path.mkdir()
                else:
                    os.mkfifo(path)
                self.create(success=False)
                self.verify(success=False)
                if kind == "directory":
                    path.rmdir()
                else:
                    path.unlink()

    def test_missing_binary_and_manifest_symlink_are_rejected(self):
        self.create()
        (self.directory / BINARIES[-1]).unlink()
        self.verify(success=False)
        manifest_path = self.directory / "native-build.json"
        manifest_path.rename(self.root / "manifest.json")
        manifest_path.symlink_to(self.root / "manifest.json")
        self.verify(success=False)

    def test_duplicate_json_fields_and_unknown_schema_are_rejected(self):
        self.create()
        manifest_path = self.directory / "native-build.json"
        text = manifest_path.read_text(encoding="utf-8")
        manifest_path.write_text(
            text.replace(
                '"schema_version": 1', '"schema_version": 1, "schema_version": 1'
            ),
            encoding="utf-8",
        )
        self.verify(success=False)
        manifest = json.loads(text)
        manifest["schema_version"] = 2
        self.write_manifest(manifest)
        self.verify(success=False)


if __name__ == "__main__":
    unittest.main()
