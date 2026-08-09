#!/usr/bin/env python3
"""Run the 20-root certifier against a public, dependency-closed batch.

The private Shared Auth sources remain covered by their own organization tests.
This release batch intentionally uses repositories that GitHub-hosted runners can
clone without cross-organization credentials, while preserving the exact same
preflight, publication, storage, and cold-install invariants.
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


if __name__ == "__main__":
    try:
        raise SystemExit(certification.main())
    except certification.CertificationError as error:
        print(f"certification error: {error}", file=sys.stderr, flush=True)
        raise SystemExit(1)
