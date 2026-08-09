#!/usr/bin/env python3
"""Run the 20-root certifier against a public, dependency-closed batch.

The private Shared Auth sources remain covered by their own organization tests.
This release batch intentionally uses repositories that GitHub-hosted runners can
clone without cross-organization credentials, while preserving the exact same
preflight, publication, storage, and cold-install invariants.

The monorepo remains a composition package. Its pinned public Git submodules are
materialized before packing; this does not start or deploy any submodule service,
and the acceptance plane still runs only zed-api-server.rs plus zed-cli.
"""

from __future__ import annotations

import sys

import certify_20_package_batch as certification

certification.REPOSITORIES = (
    "zed-pkg/zed-interfaces",
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
    "zed-pkg/zed-visual-studio",
    "zed-pkg/zed-jetbrains-air",
    "opto-sync/syncer.c",
    "opto-sync/syncer.rs",
)

_original_clone_roots = certification.clone_roots


def clone_roots_with_pinned_submodules() -> list[certification.RootPackage]:
    roots = _original_clone_roots()
    for root in roots:
        if not (root.path / ".gitmodules").is_file():
            continue
        print(f"== materialize pinned submodules for {root.repository} ==", flush=True)
        certification.run(
            ["git", "submodule", "sync", "--recursive"],
            cwd=root.path,
        )
        certification.run(
            [
                "git",
                "submodule",
                "update",
                "--init",
                "--recursive",
                "--depth",
                "1",
            ],
            cwd=root.path,
        )
        status = certification.run(
            ["git", "submodule", "status", "--recursive"],
            cwd=root.path,
            echo=False,
        )
        uninitialized = [
            line for line in status.splitlines() if line.startswith("-")
        ]
        if uninitialized:
            raise certification.CertificationError(
                f"{root.repository}: uninitialized submodules remain: {uninitialized}"
            )
    return roots


certification.clone_roots = clone_roots_with_pinned_submodules


if __name__ == "__main__":
    try:
        raise SystemExit(certification.main())
    except certification.CertificationError as error:
        print(f"certification error: {error}", file=sys.stderr, flush=True)
        raise SystemExit(1)
