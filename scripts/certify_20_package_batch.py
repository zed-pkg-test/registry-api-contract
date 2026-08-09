#!/usr/bin/env python3
"""Certify a dependency-complete 20-root Zed package batch.

The controller deliberately exercises only zed-api-server.rs and zed-cli.
It preflights every emitted polyglot coordinate before mutating the registry,
publishes exact repository heads, verifies S3-served bytes, and proves the
manifestless install -> uninstall -> cold frozen reinstall lifecycle.
"""

from __future__ import annotations

import hashlib
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tomllib
import urllib.parse
import urllib.request
from dataclasses import dataclass
from typing import Any, Iterable

REPOSITORIES: tuple[str, ...] = (
    "shared-auth/shared-auth-interfaces",
    "shared-auth/shared-auth-lib",
    "ORESoftware/k8s-libs-and-shared-defs",
    "zed-pkg/zed-interfaces",
    "zed-pkg/zed-lib-core",
    "zed-pkg/zed-clients",
    "zed-pkg/zed-lock",
    "zed-pkg/zed-cli",
    "zed-pkg/zed-api-server.rs",
    "zed-pkg/zed-sync",
    "zed-pkg/zed-docs",
    "zed-pkg/zed-e2e",
    "zed-pkg/zed-infra",
    "zed-pkg/zed-monorepo",
    "zed-pkg/zed-vscode",
    "zed-pkg/zed-intellij",
    "zed-pkg/zed-sublimetext",
    "zed-pkg/zed-eclipse",
    "zed-pkg/zed-xcode",
    "zed-pkg/zed-qtcreator",
)

EXPECTED_ROOT_COUNT = 20
ANSI_ESCAPE = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]")
PACKED_LINE = re.compile(
    r"^packed\s+"
    r"(?P<coordinate>[A-Za-z0-9._-]+/[A-Za-z0-9._-]+)"
    r"@(?P<version>\S+?)"
    r"(?:\s+\(target\s+(?P<target>[^)]+)\))?\s*$"
)
SHA_LINE = re.compile(r"\bsha256\b[\s:=]+(?P<sha>[0-9a-fA-F]{64})\b")


class CertificationError(RuntimeError):
    """A fail-closed batch certification error."""


@dataclass(frozen=True)
class RootPackage:
    repository: str
    path: pathlib.Path
    source_sha: str
    branch: str
    org: str
    name: str
    version: str
    tag: str
    dependencies: tuple[str, ...]

    @property
    def coordinate(self) -> str:
        return f"{self.org}/{self.name}"

    @property
    def identity(self) -> str:
        return f"{self.coordinate}@{self.version}"

    def as_json(self) -> dict[str, Any]:
        return {
            "repository": self.repository,
            "path": str(self.path),
            "source_sha": self.source_sha,
            "branch": self.branch,
            "org": self.org,
            "name": self.name,
            "version": self.version,
            "tag": self.tag,
            "dependencies": list(self.dependencies),
            "coordinate": self.coordinate,
            "identity": self.identity,
        }


@dataclass(frozen=True)
class EmittedPackage:
    repository: str
    source_sha: str
    root_identity: str
    coordinate: str
    version: str
    target: str
    artifact_path: str
    artifact_sha256: str

    @property
    def org(self) -> str:
        return self.coordinate.split("/", 1)[0]

    @property
    def name(self) -> str:
        return self.coordinate.split("/", 1)[1]

    @property
    def identity(self) -> str:
        return f"{self.coordinate}@{self.version}"

    def as_json(self) -> dict[str, Any]:
        return {
            "repository": self.repository,
            "source_sha": self.source_sha,
            "root_identity": self.root_identity,
            "coordinate": self.coordinate,
            "version": self.version,
            "target": self.target,
            "artifact_path": self.artifact_path,
            "artifact_sha256": self.artifact_sha256,
            "identity": self.identity,
        }


def required_env(name: str) -> str:
    value = os.environ.get(name, "").strip()
    if not value:
        raise CertificationError(f"required environment variable {name} is empty")
    return value


WORKSPACE = pathlib.Path(required_env("GITHUB_WORKSPACE")).resolve()
EVIDENCE = WORKSPACE / "evidence-v2"
SOURCES = WORKSPACE / "sources-v2"
CONSUMERS = WORKSPACE / "consumers-v2"
HOMES = WORKSPACE / ".zed-homes-v2"
ZED = pathlib.Path(required_env("ZED")).resolve()
REGISTRY = required_env("ZED_PKG_REGISTRY").rstrip("/")
TOKEN = required_env("ZED_PKG_TOKEN")
API_SHA = required_env("ZED_API_SHA")
CLI_SHA = required_env("ZED_CLI_SHA")


def scrubbed_environment(extra: dict[str, str] | None = None) -> dict[str, str]:
    env = os.environ.copy()
    env.update(
        {
            "NO_COLOR": "1",
            "CLICOLOR": "0",
            "ZED_PKG_REGISTRY": REGISTRY,
            "ZED_PKG_TOKEN": TOKEN,
            "ZED_PKG_INSTALL_MODE": "copy",
            "ZED_PKG_ALLOW_NO_MANIFEST": "1",
            "ZED_PKG_ALLOW_ECOSYSTEM_MISMATCH": "1",
        }
    )
    if extra:
        env.update(extra)
    return env


def clean_text(text: str) -> str:
    return ANSI_ESCAPE.sub("", text).replace("\r\n", "\n")


def run(
    args: Iterable[str | os.PathLike[str]],
    *,
    cwd: pathlib.Path | None = None,
    env: dict[str, str] | None = None,
    log_path: pathlib.Path | None = None,
    echo: bool = True,
    check: bool = True,
) -> str:
    argv = [os.fspath(value) for value in args]
    proc = subprocess.run(
        argv,
        cwd=cwd,
        env=env or scrubbed_environment(),
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        check=False,
    )
    output = clean_text(proc.stdout or "")
    if log_path is not None:
        log_path.parent.mkdir(parents=True, exist_ok=True)
        with log_path.open("a", encoding="utf-8") as handle:
            handle.write(f"$ {' '.join(argv)}\n")
            handle.write(output)
            if output and not output.endswith("\n"):
                handle.write("\n")
            handle.write(f"[exit {proc.returncode}]\n")
    if echo and output:
        print(output, end="" if output.endswith("\n") else "\n", flush=True)
    if check and proc.returncode != 0:
        location = f" in {cwd}" if cwd is not None else ""
        raise CertificationError(
            f"command failed with exit {proc.returncode}{location}: {' '.join(argv)}"
        )
    return output


def write_json(path: pathlib.Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(value, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def read_public_probe() -> list[dict[str, str]]:
    path = EVIDENCE / "public-api-probe.tsv"
    rows: list[dict[str, str]] = []
    if not path.is_file():
        return rows
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        if not raw_line.strip():
            continue
        parts = raw_line.split("\t")
        if len(parts) < 2:
            raise CertificationError(f"malformed public probe line: {raw_line!r}")
        rows.append(
            {
                "url": parts[0],
                "status": parts[1],
                "final_url": parts[2] if len(parts) > 2 else "",
            }
        )
    return rows


def clone_roots() -> list[RootPackage]:
    roots: list[RootPackage] = []
    source_rows: list[dict[str, Any]] = []
    for repository in REPOSITORIES:
        slug = repository.replace("/", "__")
        path = SOURCES / slug
        print(f"== clone {repository} ==", flush=True)
        run(
            [
                "git",
                "clone",
                "--quiet",
                "--depth",
                "1",
                f"https://github.com/{repository}.git",
                path,
            ]
        )
        source_sha = run(
            ["git", "rev-parse", "HEAD"], cwd=path, echo=False
        ).strip()
        branch = run(
            ["git", "branch", "--show-current"], cwd=path, echo=False
        ).strip()
        manifest_path = path / ".zpkg.toml"
        if not manifest_path.is_file():
            raise CertificationError(f"{repository} has no .zpkg.toml at {source_sha}")
        manifest = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
        package = manifest.get("package")
        if not isinstance(package, dict):
            raise CertificationError(f"{repository}: missing [package]")
        org = package.get("org")
        name = package.get("name")
        version = package.get("version")
        if not all(isinstance(value, str) and value.strip() for value in (org, name, version)):
            raise CertificationError(
                f"{repository}: invalid non-empty package org/name/version"
            )
        dependencies_section = manifest.get("dependencies") or {}
        if not isinstance(dependencies_section, dict):
            raise CertificationError(f"{repository}: [dependencies] must be a table")
        dependencies = tuple(sorted(str(key) for key in dependencies_section))
        publish = manifest.get("publish") or {}
        if not isinstance(publish, dict):
            raise CertificationError(f"{repository}: [publish] must be a table")
        tag_format = publish.get("tag_format", "v{version}")
        if not isinstance(tag_format, str) or "{version}" not in tag_format:
            raise CertificationError(
                f"{repository}: publish.tag_format must contain {{version}}"
            )
        tag = tag_format.replace("{version}", version)
        root = RootPackage(
            repository=repository,
            path=path,
            source_sha=source_sha,
            branch=branch,
            org=org,
            name=name,
            version=version,
            tag=tag,
            dependencies=dependencies,
        )
        roots.append(root)
        source_rows.append(root.as_json())

        if run(["git", "status", "--porcelain"], cwd=path, echo=False).strip():
            raise CertificationError(f"{repository}: dirty immediately after clone")
        run(["git", "config", "user.name", "Zed Batch Publisher"], cwd=path)
        run(
            ["git", "config", "user.email", "zed-batch@example.invalid"],
            cwd=path,
        )
        run(["git", "tag", "--force", tag, source_sha], cwd=path)

    if len(roots) != EXPECTED_ROOT_COUNT:
        raise CertificationError(
            f"expected {EXPECTED_ROOT_COUNT} source roots, got {len(roots)}"
        )

    coordinates = [root.coordinate for root in roots]
    if len(set(coordinates)) != len(coordinates):
        duplicates = sorted(
            coordinate
            for coordinate in set(coordinates)
            if coordinates.count(coordinate) > 1
        )
        raise CertificationError(f"duplicate root coordinates: {duplicates}")

    positions = {root.coordinate: index for index, root in enumerate(roots)}
    for index, root in enumerate(roots):
        for dependency in root.dependencies:
            if dependency not in positions:
                raise CertificationError(
                    f"{root.identity}: dependency {dependency} is absent from the 20-root closure"
                )
            if positions[dependency] >= index:
                raise CertificationError(
                    f"{root.identity}: dependency {dependency} is not published earlier "
                    "in the topological batch order"
                )

    write_json(EVIDENCE / "source-roots.json", source_rows)
    return roots


def parse_pack_output(root: RootPackage, output: str) -> list[EmittedPackage]:
    lines = clean_text(output).splitlines()
    emitted: list[EmittedPackage] = []
    for index, raw_line in enumerate(lines):
        match = PACKED_LINE.match(raw_line.strip())
        if match is None:
            continue
        artifact_path = ""
        artifact_sha = ""
        for follower in lines[index + 1 : index + 7]:
            stripped = follower.strip()
            sha_match = SHA_LINE.search(stripped)
            if sha_match is not None:
                artifact_sha = sha_match.group("sha").lower()
                break
            if stripped and not artifact_path:
                artifact_path = stripped
        if not artifact_sha:
            raise CertificationError(
                f"{root.repository}: packed identity on line {index + 1} has no sha256"
            )
        emitted.append(
            EmittedPackage(
                repository=root.repository,
                source_sha=root.source_sha,
                root_identity=root.identity,
                coordinate=match.group("coordinate"),
                version=match.group("version"),
                target=(match.group("target") or "root").strip(),
                artifact_path=artifact_path,
                artifact_sha256=artifact_sha,
            )
        )
    if not emitted:
        raise CertificationError(
            f"{root.repository}: `zed pack` emitted no parseable package identities"
        )
    return emitted


def preflight_all(roots: list[RootPackage]) -> list[EmittedPackage]:
    emitted: list[EmittedPackage] = []
    pack_log = EVIDENCE / "preflight-pack.log"
    for root in roots:
        print(f"== preflight {root.repository}@{root.source_sha} ==", flush=True)
        output = run(
            [ZED, "pack"],
            cwd=root.path,
            log_path=pack_log,
        )
        root_emitted = parse_pack_output(root, output)
        if root.identity not in {package.identity for package in root_emitted}:
            identities = sorted(package.identity for package in root_emitted)
            raise CertificationError(
                f"{root.repository}: canonical root {root.identity} was not emitted; "
                f"got {identities}"
            )
        emitted.extend(root_emitted)
        shutil.rmtree(root.path / ".zed", ignore_errors=True)
        if run(
            ["git", "status", "--porcelain"], cwd=root.path, echo=False
        ).strip():
            raise CertificationError(
                f"{root.repository}: zed pack changed tracked repository state"
            )

    by_identity: dict[str, EmittedPackage] = {}
    for package in emitted:
        previous = by_identity.get(package.identity)
        if previous is not None:
            raise CertificationError(
                "duplicate emitted immutable identity before upload: "
                f"{package.identity} from {previous.repository} and {package.repository}"
            )
        by_identity[package.identity] = package

    write_json(
        EVIDENCE / "emitted-preflight.json",
        [package.as_json() for package in emitted],
    )
    print(
        f"preflighted {len(roots)} roots and {len(emitted)} unique emitted coordinates",
        flush=True,
    )
    return emitted


def claim_orgs(roots: list[RootPackage]) -> None:
    log_path = EVIDENCE / "org-claim.log"
    for org in sorted({root.org for root in roots}):
        print(f"== claim organization {org} ==", flush=True)
        run([ZED, "org", "claim", org], log_path=log_path)


def publish_all(
    roots: list[RootPackage],
    *,
    log_name: str,
) -> list[dict[str, str]]:
    results: list[dict[str, str]] = []
    log_path = EVIDENCE / log_name
    for root in roots:
        if (
            run(
                ["git", "rev-parse", "HEAD"],
                cwd=root.path,
                echo=False,
            ).strip()
            != root.source_sha
        ):
            raise CertificationError(f"{root.repository}: source head drifted")
        if run(
            ["git", "status", "--porcelain"], cwd=root.path, echo=False
        ).strip():
            raise CertificationError(f"{root.repository}: source became dirty")
        print(f"== publish {root.identity} from {root.repository}@{root.source_sha} ==", flush=True)
        output = run(
            [ZED, "publish"],
            cwd=root.path,
            log_path=log_path,
        )
        results.append(
            {
                "repository": root.repository,
                "source_sha": root.source_sha,
                "root_identity": root.identity,
                "output_sha256": hashlib.sha256(output.encode("utf-8")).hexdigest(),
            }
        )
        shutil.rmtree(root.path / ".zed", ignore_errors=True)
        if run(
            ["git", "status", "--porcelain"], cwd=root.path, echo=False
        ).strip():
            raise CertificationError(
                f"{root.repository}: publish changed tracked repository state"
            )
    return results


def http_json(url: str) -> dict[str, Any]:
    request = urllib.request.Request(
        url,
        headers={"Accept": "application/json", "User-Agent": "zed-batch-certifier/2"},
    )
    with urllib.request.urlopen(request, timeout=60) as response:
        if response.status != 200:
            raise CertificationError(f"GET {url} returned HTTP {response.status}")
        value = json.loads(response.read().decode("utf-8"))
    if not isinstance(value, dict):
        raise CertificationError(f"GET {url} did not return a JSON object")
    return value


def download_and_hash(url: str) -> tuple[str, int, str]:
    request = urllib.request.Request(
        url,
        headers={"User-Agent": "zed-batch-certifier/2"},
    )
    digest = hashlib.sha256()
    size = 0
    with urllib.request.urlopen(request, timeout=180) as response:
        final_url = response.geturl()
        while True:
            chunk = response.read(1024 * 1024)
            if not chunk:
                break
            digest.update(chunk)
            size += len(chunk)
    return digest.hexdigest(), size, final_url


def metadata_url(package: EmittedPackage) -> str:
    return (
        f"{REGISTRY}/v1/packages/"
        f"{urllib.parse.quote(package.org, safe='')}/"
        f"{urllib.parse.quote(package.name, safe='')}/versions/"
        f"{urllib.parse.quote(package.version, safe='')}"
    )


def verify_registry(
    emitted: list[EmittedPackage],
    *,
    save_name: str,
    download: bool,
) -> list[dict[str, Any]]:
    metadata_dir = EVIDENCE / "metadata"
    metadata_dir.mkdir(parents=True, exist_ok=True)
    verified: list[dict[str, Any]] = []
    for package in emitted:
        print(f"== verify {package.identity} ==", flush=True)
        meta = http_json(metadata_url(package))
        for field, expected in (
            ("org", package.org),
            ("name", package.name),
            ("version", package.version),
        ):
            if meta.get(field) != expected:
                raise CertificationError(
                    f"{package.identity}: metadata {field}={meta.get(field)!r}, "
                    f"expected {expected!r}"
                )
        sha = meta.get("sha256")
        if sha != package.artifact_sha256:
            raise CertificationError(
                f"{package.identity}: registry sha {sha!r} != preflight "
                f"{package.artifact_sha256}"
            )
        if meta.get("yanked") is not False:
            raise CertificationError(f"{package.identity}: published version is yanked")
        artifact_format = meta.get("format")
        download_url = meta.get("download_url")
        if not isinstance(artifact_format, str) or not artifact_format:
            raise CertificationError(f"{package.identity}: missing artifact format")
        if not isinstance(download_url, str) or not download_url:
            raise CertificationError(f"{package.identity}: missing download_url")
        download_url = urllib.parse.urljoin(REGISTRY + "/", download_url)

        downloaded_sha = ""
        downloaded_size = 0
        final_url = ""
        s3_verified = False
        if download:
            downloaded_sha, downloaded_size, final_url = download_and_hash(download_url)
            if downloaded_sha != sha:
                raise CertificationError(
                    f"{package.identity}: downloaded sha {downloaded_sha} != metadata {sha}"
                )
            parsed = urllib.parse.urlparse(final_url)
            s3_verified = (
                parsed.hostname in {"127.0.0.1", "localhost"}
                and parsed.port == 19000
                and "/zed-pkg-artifacts/artifacts/" in parsed.path
                and f"{sha}.{artifact_format}" in parsed.path
            )
            if not s3_verified:
                raise CertificationError(
                    f"{package.identity}: download did not terminate at the expected "
                    f"S3-compatible object URL: {final_url}"
                )

        metadata_path = (
            metadata_dir
            / f"{package.org}__{package.name}__{package.version}.json"
        )
        write_json(metadata_path, meta)
        verified.append(
            {
                **package.as_json(),
                "metadata_url": metadata_url(package),
                "metadata_sha256": sha,
                "format": artifact_format,
                "download_url": download_url,
                "downloaded_sha256": downloaded_sha,
                "downloaded_size": downloaded_size,
                "final_object_url": final_url,
                "metadata_verified": True,
                "download_verified": download and downloaded_sha == sha,
                "s3_object_verified": download and s3_verified,
            }
        )
    write_json(EVIDENCE / save_name, verified)
    return verified


def installed_manifest_files(consumer: pathlib.Path) -> list[str]:
    modules = consumer / "zed_modules"
    if not modules.exists():
        return []
    return sorted(
        str(path.relative_to(consumer))
        for path in modules.rglob(".zpkg.toml")
        if path.is_file()
    )


def installed_payload_files(consumer: pathlib.Path) -> list[pathlib.Path]:
    modules = consumer / "zed_modules"
    if not modules.exists():
        return []
    return [
        path
        for path in modules.rglob("*")
        if path.is_file() or path.is_symlink()
    ]


def certify_installs(roots: list[RootPackage]) -> list[dict[str, Any]]:
    install_log = EVIDENCE / "install-lifecycle.log"
    lock_dir = EVIDENCE / "locks"
    lock_dir.mkdir(parents=True, exist_ok=True)
    results: list[dict[str, Any]] = []

    for index, root in enumerate(roots, start=1):
        print(f"== install lifecycle {root.identity} ==", flush=True)
        consumer = CONSUMERS / f"{index:02d}-{root.org}__{root.name}"
        home = HOMES / f"{index:02d}-{root.org}__{root.name}"
        consumer.mkdir(parents=True, exist_ok=True)
        shutil.rmtree(home, ignore_errors=True)
        home.mkdir(parents=True, exist_ok=True)
        env = scrubbed_environment({"ZED_PKG_HOME": str(home)})
        common_flags = [
            "--skip-manifest",
            "--install-mode",
            "copy",
            "--adapter",
            "none",
            "--allow-build",
            "--allow-ecosystem-mismatch",
        ]

        run(
            [ZED, "install", root.identity, *common_flags],
            cwd=consumer,
            env=env,
            log_path=install_log,
        )
        lock_path = consumer / ".zpkg.lock"
        if not lock_path.is_file() or lock_path.stat().st_size == 0:
            raise CertificationError(f"{root.identity}: install produced no lockfile")
        first_lock = lock_path.read_bytes()
        first_manifest_files = installed_manifest_files(consumer)
        if not first_manifest_files:
            raise CertificationError(
                f"{root.identity}: install produced no materialized package manifests"
            )
        first_lock_path = lock_dir / f"{root.org}__{root.name}.first.lock"
        first_lock_path.write_bytes(first_lock)

        run(
            [ZED, "uninstall"],
            cwd=consumer,
            env=env,
            log_path=install_log,
        )
        if installed_payload_files(consumer):
            remaining = [
                str(path.relative_to(consumer))
                for path in installed_payload_files(consumer)[:20]
            ]
            raise CertificationError(
                f"{root.identity}: uninstall left payload files: {remaining}"
            )
        if not lock_path.is_file() or lock_path.read_bytes() != first_lock:
            raise CertificationError(
                f"{root.identity}: uninstall did not retain the exact lockfile"
            )

        shutil.rmtree(home, ignore_errors=True)
        home.mkdir(parents=True, exist_ok=True)
        run(
            [ZED, "install", "--frozen", *common_flags],
            cwd=consumer,
            env=env,
            log_path=install_log,
        )
        frozen_lock = lock_path.read_bytes()
        if frozen_lock != first_lock:
            raise CertificationError(
                f"{root.identity}: cold frozen reinstall changed the lockfile"
            )
        frozen_manifest_files = installed_manifest_files(consumer)
        if not frozen_manifest_files:
            raise CertificationError(
                f"{root.identity}: frozen reinstall materialized no package manifests"
            )
        frozen_lock_path = lock_dir / f"{root.org}__{root.name}.frozen.lock"
        frozen_lock_path.write_bytes(frozen_lock)

        results.append(
            {
                **root.as_json(),
                "initial_materialized_manifests": first_manifest_files,
                "frozen_materialized_manifests": frozen_manifest_files,
                "lock_sha256": hashlib.sha256(first_lock).hexdigest(),
                "install_verified": True,
                "uninstall_verified": True,
                "cold_frozen_reinstall_verified": True,
            }
        )

    write_json(EVIDENCE / "root-install-lifecycle.json", results)
    return results


def certify_idempotent_republish(
    roots: list[RootPackage],
    emitted: list[EmittedPackage],
    before: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    before_sha = {
        row["identity"]: row["metadata_sha256"]
        for row in before
    }
    publish_all(roots, log_name="idempotent-republish.log")
    after = verify_registry(
        emitted,
        save_name="registry-after-republish.json",
        download=False,
    )
    after_sha = {
        row["identity"]: row["metadata_sha256"]
        for row in after
    }
    if before_sha != after_sha:
        raise CertificationError(
            "idempotent republish changed one or more immutable metadata hashes"
        )
    return after


def main() -> int:
    for path in (EVIDENCE, SOURCES, CONSUMERS, HOMES):
        shutil.rmtree(path, ignore_errors=True)
        path.mkdir(parents=True, exist_ok=True)

    # The workflow writes this probe before invoking the controller. Preserve it
    # across the evidence directory reset by moving it through RUNNER_TEMP.
    probe_backup = os.environ.get("PUBLIC_PROBE_BACKUP", "").strip()
    if probe_backup:
        backup_path = pathlib.Path(probe_backup)
        if backup_path.is_file():
            shutil.copy2(backup_path, EVIDENCE / "public-api-probe.tsv")

    if not ZED.is_file():
        raise CertificationError(f"ZED binary does not exist: {ZED}")
    actual_cli_sha = run(
        ["git", "-C", WORKSPACE / "upstream/zed-cli", "rev-parse", "HEAD"],
        echo=False,
    ).strip()
    actual_api_sha = run(
        ["git", "-C", WORKSPACE / "upstream/zed-api-server", "rev-parse", "HEAD"],
        echo=False,
    ).strip()
    if actual_cli_sha != CLI_SHA:
        raise CertificationError(f"CLI exact-head mismatch: {actual_cli_sha} != {CLI_SHA}")
    if actual_api_sha != API_SHA:
        raise CertificationError(f"API exact-head mismatch: {actual_api_sha} != {API_SHA}")

    roots = clone_roots()
    emitted = preflight_all(roots)
    claim_orgs(roots)
    first_publications = publish_all(roots, log_name="initial-publish.log")
    registry_rows = verify_registry(
        emitted,
        save_name="registry-verification.json",
        download=True,
    )
    install_rows = certify_installs(roots)
    after_republish = certify_idempotent_republish(
        roots,
        emitted,
        registry_rows,
    )

    install_by_identity = {row["identity"]: row for row in install_rows}
    emitted_by_identity = {row["identity"]: row for row in registry_rows}
    summary = {
        "schema": "zed.package-batch-certification/v2",
        "api_source_sha": API_SHA,
        "cli_source_sha": CLI_SHA,
        "api_plane_only": True,
        "zed_web_server_used": False,
        "metadata_database": "postgres/pgvector",
        "artifact_backend": "s3-compatible/minio",
        "source_repository_count": len(roots),
        "root_package_count": len(roots),
        "emitted_package_count": len(emitted),
        "public_api_probe": read_public_probe(),
        "all_emitted_identities_unique": len(
            {package.identity for package in emitted}
        )
        == len(emitted),
        "all_metadata_verified": all(
            row["metadata_verified"] for row in registry_rows
        ),
        "all_downloads_verified": all(
            row["download_verified"] for row in registry_rows
        ),
        "all_s3_objects_verified": all(
            row["s3_object_verified"] for row in registry_rows
        ),
        "all_roots_installed": all(
            row["install_verified"] for row in install_rows
        ),
        "all_roots_uninstalled": all(
            row["uninstall_verified"] for row in install_rows
        ),
        "all_roots_cold_frozen_reinstalled": all(
            row["cold_frozen_reinstall_verified"] for row in install_rows
        ),
        "idempotent_republish_verified": {
            row["identity"]: row["metadata_sha256"]
            for row in after_republish
        }
        == {
            row["identity"]: row["metadata_sha256"]
            for row in registry_rows
        },
        "roots": [
            {
                **root.as_json(),
                "install": install_by_identity[root.identity],
            }
            for root in roots
        ],
        "emitted_packages": [
            emitted_by_identity[package.identity]
            for package in emitted
        ],
        "initial_publications": first_publications,
    }

    required_true = (
        "api_plane_only",
        "all_emitted_identities_unique",
        "all_metadata_verified",
        "all_downloads_verified",
        "all_s3_objects_verified",
        "all_roots_installed",
        "all_roots_uninstalled",
        "all_roots_cold_frozen_reinstalled",
        "idempotent_republish_verified",
    )
    if summary["source_repository_count"] != EXPECTED_ROOT_COUNT:
        raise CertificationError("source repository count drifted")
    if summary["root_package_count"] != EXPECTED_ROOT_COUNT:
        raise CertificationError("root package count drifted")
    if not all(bool(summary[key]) for key in required_true):
        failed = [key for key in required_true if not summary[key]]
        raise CertificationError(f"summary invariants failed: {failed}")

    write_json(EVIDENCE / "batch-certification.json", summary)
    print(
        json.dumps(
            {
                "source_repository_count": summary["source_repository_count"],
                "root_package_count": summary["root_package_count"],
                "emitted_package_count": summary["emitted_package_count"],
                "api_source_sha": summary["api_source_sha"],
                "cli_source_sha": summary["cli_source_sha"],
                "all_metadata_verified": summary["all_metadata_verified"],
                "all_s3_objects_verified": summary["all_s3_objects_verified"],
                "all_roots_installed": summary["all_roots_installed"],
                "all_roots_cold_frozen_reinstalled": summary[
                    "all_roots_cold_frozen_reinstalled"
                ],
                "idempotent_republish_verified": summary[
                    "idempotent_republish_verified"
                ],
                "public_api_probe": summary["public_api_probe"],
            },
            indent=2,
            sort_keys=True,
        ),
        flush=True,
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except CertificationError as error:
        print(f"certification error: {error}", file=sys.stderr, flush=True)
        raise SystemExit(1)
