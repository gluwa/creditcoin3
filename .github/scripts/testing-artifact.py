#!/usr/bin/env python3
"""Create and verify provenance for CI testing binaries (Python standard library only).

The build section describes the original compilation and survives a verified cache
hit. The producer section describes this artifact upload. GitHub API metadata is
supplied by the workflow; this helper never obtains credentials or downloads files.
"""

import argparse
import hashlib
import json
import os
import re
import stat
import sys
import tempfile
from pathlib import Path

MANIFEST = "native-build.json"
REPOSITORY = "gluwa/creditcoin3"
ARTIFACT = "binary-for-testing-ci"
WORKFLOW = ".github/workflows/ci.yml"
BINARIES = (
    "archiver",
    "attestor",
    "attestor_zombienet",
    "checkpoint-builder",
    "creditcoin3-node",
    "proof-gen-api-server",
    "query-cli",
)
SETTINGS = {
    "build_options": "--release --features=fast-runtime",
    "workspace_opt_level": "1",
    "runner_os": "Linux",
    "runner_arch": "X64",
}


class ProvenanceError(ValueError):
    """The artifact does not satisfy the required provenance contract."""


def require(condition, message):
    if not condition:
        raise ProvenanceError(message)


def positive(value, label):
    require(type(value) is int and value > 0, f"{label} must be a positive integer")
    return value


def source_sha(value, label):
    require(
        isinstance(value, str) and re.fullmatch(r"[0-9a-f]{40}", value),
        f"invalid {label}",
    )
    return value


def sha256(value, label):
    require(
        isinstance(value, str) and re.fullmatch(r"[0-9a-f]{64}", value),
        f"invalid {label}",
    )
    return value


def repository_name(value, label):
    require(
        isinstance(value, str)
        and re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", value),
        f"invalid {label}",
    )


def object_fields(value, fields, label):
    require(isinstance(value, dict), f"{label} must be an object")
    require(set(value) == set(fields), f"unexpected {label} fields")


def no_duplicate_keys(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, f"duplicate JSON field: {key}")
        result[key] = value
    return result


def regular_file(path):
    # NONBLOCK prevents a FIFO from hanging before fstat rejects it; NOFOLLOW
    # rejects symlinks, including a swapped-in link between lookup and open.
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        require(stat.S_ISREG(os.fstat(fd).st_mode), f"not a regular file: {path}")
        return os.fdopen(fd, "rb")
    except BaseException:
        os.close(fd)
        raise


def read_json(path):
    with regular_file(path) as stream:
        value = json.load(stream, object_pairs_hook=no_duplicate_keys)
    require(isinstance(value, dict), f"JSON object required: {path}")
    return value


def binary_hashes(directory):
    require(
        stat.S_ISDIR(directory.lstat().st_mode),
        "binary directory must be a real directory",
    )
    result = {}
    for name in BINARIES:
        digest = hashlib.sha256()
        with regular_file(directory / name) as stream:
            for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                digest.update(chunk)
        result[name] = digest.hexdigest()
    return result


def validate_manifest(manifest):
    object_fields(manifest, ("schema_version", "build", "producer"), "manifest")
    require(
        type(manifest["schema_version"]) is int and manifest["schema_version"] == 1,
        "unsupported schema",
    )
    build = manifest["build"]
    object_fields(
        build,
        ("repository", "source_sha", "source_key", "binaries", *SETTINGS),
        "build",
    )
    repository_name(build["repository"], "build repository")
    source_sha(build["source_sha"], "build source SHA")
    require(
        isinstance(build["source_key"], str) and build["source_key"].strip(),
        "missing source key",
    )
    for name in SETTINGS:
        require(isinstance(build[name], str), f"invalid build setting: {name}")
        require(
            build[name] or name == "workspace_opt_level",
            f"missing build setting: {name}",
        )
    object_fields(build["binaries"], BINARIES, "binary hashes")
    for name, digest in build["binaries"].items():
        sha256(digest, f"{name} hash")
    producer = manifest["producer"]
    object_fields(
        producer, ("repository", "source_sha", "run_id", "run_attempt"), "producer"
    )
    repository_name(producer["repository"], "producer repository")
    source_sha(producer["source_sha"], "producer source SHA")
    positive(producer["run_id"], "producer run ID")
    positive(producer["run_attempt"], "producer run attempt")


def write_manifest(directory, manifest):
    # Replace the manifest only after every cache/build check has succeeded.
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="w", encoding="utf-8", dir=directory, delete=False
        ) as stream:
            temporary = stream.name
            json.dump(manifest, stream, indent=2, sort_keys=True)
            stream.write("\n")
        os.replace(temporary, directory / MANIFEST)
    finally:
        if temporary and os.path.exists(temporary):
            os.unlink(temporary)


def create(args):
    directory = Path(args.directory)
    producer = {
        "repository": args.repository,
        "source_sha": args.source_sha,
        "run_id": args.run_id,
        "run_attempt": args.run_attempt,
    }
    settings = {name: getattr(args, name) for name in SETTINGS}
    hashes = binary_hashes(directory)
    if args.cache_hit == "true":
        manifest = read_json(directory / MANIFEST)
        validate_manifest(manifest)
        build = manifest["build"]
        require(
            build["repository"] == args.source_repository,
            "cached source repository mismatch",
        )
        require(build["source_key"] == args.source_key, "cached source key mismatch")
        require(
            all(build[name] == value for name, value in settings.items()),
            "cached build settings mismatch",
        )
        require(build["binaries"] == hashes, "cached binary hash mismatch")
    else:
        build = {
            "repository": args.source_repository,
            "source_sha": args.source_sha,
            "source_key": args.source_key,
            **settings,
            "binaries": hashes,
        }
    manifest = {"schema_version": 1, "build": build, "producer": producer}
    validate_manifest(manifest)
    write_manifest(directory, manifest)
    return manifest


def validate_metadata(args):
    artifact = read_json(args.artifact_metadata)
    run = read_json(args.run_metadata)
    positive(args.expected_artifact_id, "expected artifact ID")
    positive(args.expected_run_id, "expected run ID")
    positive(args.current_run_id, "current run ID")
    expected_sha = source_sha(args.expected_source_sha, "expected source SHA")
    require(
        type(artifact.get("id")) is int and artifact["id"] == args.expected_artifact_id,
        "artifact ID mismatch",
    )
    require(artifact.get("name") == ARTIFACT, "unexpected artifact name")
    require(
        artifact.get("expired") is False, "artifact expired or missing expiry metadata"
    )
    digest = artifact.get("digest")
    require(
        isinstance(digest, str) and digest.startswith("sha256:"),
        "missing artifact SHA-256 digest",
    )
    sha256(digest[7:], "artifact digest")
    require(
        type(run.get("id")) is int and run["id"] == args.expected_run_id,
        "run ID mismatch",
    )
    require(run.get("path") == WORKFLOW, "unexpected producer workflow")
    repository = run.get("repository", {})
    require(
        isinstance(repository, dict) and repository.get("full_name") == REPOSITORY,
        "unexpected run repository",
    )
    repository_id = positive(repository.get("id"), "run repository ID")
    head = source_sha(run.get("head_sha"), "run head SHA")
    positive(run.get("run_attempt"), "run attempt")
    link = artifact.get("workflow_run", {})
    require(isinstance(link, dict), "missing artifact workflow run")
    require(
        type(link.get("id")) is int and link["id"] == run["id"],
        "artifact/run linkage mismatch",
    )
    require(
        type(link.get("repository_id")) is int
        and link["repository_id"] == repository_id,
        "artifact repository mismatch",
    )
    require(link.get("head_sha") == head, "artifact/run head mismatch")
    same_run = (
        args.current_repository == REPOSITORY and args.current_run_id == run["id"]
    )
    completed = run.get("status") == "completed" and run.get("conclusion") == "success"
    active = (
        same_run
        and run.get("status") == "in_progress"
        and run.get("conclusion") is None
    )
    require(
        completed or active,
        "producer run must have succeeded (or be the current in-progress CC3 run)",
    )
    if args.source_commit_metadata:
        commit = read_json(args.source_commit_metadata)
        require(
            commit.get("sha") == expected_sha, "source commit metadata SHA mismatch"
        )
    else:
        commit = None
    if expected_sha != head:
        require(commit is not None, "merge source requires source commit metadata")
        parents = commit.get("parents")
        require(
            isinstance(parents, list) and len(parents) == 2,
            "source must be a direct two-parent merge",
        )
        parent_shas = [
            source_sha(
                parent.get("sha") if isinstance(parent, dict) else None,
                "merge parent SHA",
            )
            for parent in parents
        ]
        require(
            len(set(parent_shas)) == 2 and head in parent_shas,
            "source merge does not contain the producer run head",
        )
    return {
        "artifact_id": artifact["id"],
        "digest": digest,
        "run_id": run["id"],
        "run_attempt": run["run_attempt"],
    }


def verify(args):
    metadata = validate_metadata(args)
    directory = Path(args.directory)
    manifest = read_json(directory / MANIFEST)
    validate_manifest(manifest)
    producer = manifest["producer"]
    require(producer["repository"] == REPOSITORY, "unexpected producer repository")
    require(
        producer["source_sha"] == args.expected_source_sha,
        "producer source SHA mismatch",
    )
    require(producer["run_id"] == args.expected_run_id, "manifest run ID mismatch")
    require(
        producer["run_attempt"] <= metadata["run_attempt"],
        "manifest run attempt is newer than the producer run",
    )
    build = manifest["build"]
    require(build["repository"] == REPOSITORY, "unexpected build repository")
    require(
        all(build[name] == value for name, value in SETTINGS.items()),
        "incompatible testing build settings",
    )
    require(build["binaries"] == binary_hashes(directory), "binary hash mismatch")
    return {**metadata, "build": build, "producer": producer}


def metadata_arguments(parser):
    parser.add_argument("--artifact-metadata", required=True)
    parser.add_argument("--run-metadata", required=True)
    parser.add_argument("--source-commit-metadata")
    parser.add_argument("--expected-artifact-id", required=True, type=int)
    parser.add_argument("--expected-run-id", required=True, type=int)
    parser.add_argument("--expected-source-sha", required=True)
    parser.add_argument("--current-repository", required=True)
    parser.add_argument("--current-run-id", required=True, type=int)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    make = commands.add_parser(
        "create", help="write provenance after compilation or a verified cache hit"
    )
    make.add_argument("--directory", required=True)
    make.add_argument("--repository", required=True)
    make.add_argument("--source-repository", required=True)
    make.add_argument("--source-sha", required=True)
    make.add_argument("--run-id", required=True, type=int)
    make.add_argument("--run-attempt", required=True, type=int)
    make.add_argument("--source-key", required=True)
    make.add_argument("--cache-hit", choices=("true", "false"), required=True)
    for name in SETTINGS:
        make.add_argument("--" + name.replace("_", "-"), required=True)
    make.set_defaults(action=create)
    metadata = commands.add_parser(
        "validate-metadata", help="validate GitHub metadata before downloading"
    )
    metadata_arguments(metadata)
    metadata.set_defaults(action=validate_metadata)
    check = commands.add_parser(
        "verify", help="verify downloaded binaries and their provenance"
    )
    metadata_arguments(check)
    check.add_argument("--directory", required=True)
    check.set_defaults(action=verify)
    args = parser.parse_args()
    try:
        print(json.dumps(args.action(args), sort_keys=True))
    except (OSError, ValueError) as error:
        print(f"provenance validation failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
