#!/usr/bin/env python3
"""
Aloe Desktop Windows release build script.
Follows the process documented in UPDATER.md.

Usage:
    python build_release.py           # interactive version prompt
    python build_release.py 1.2.3     # pass version directly
"""

import getpass
import json
import os
import shutil
import subprocess
import sys
import urllib.parse
from pathlib import Path

SCRIPT_DIR   = Path(__file__).parent.resolve()
TAURI_CONF   = SCRIPT_DIR / "src-tauri" / "tauri.conf.json"
KEY_PATH     = SCRIPT_DIR / ".tauri" / "aloe.key"
BUNDLE_DIR   = SCRIPT_DIR / "src-tauri" / "target" / "release" / "bundle"
FRONTEND_DIR = SCRIPT_DIR.parent / "aloe-frontend" / "public" / "desktop"
FRONTEND_URL = "https://aloe.247autoarmy.in"


def read_conf():
    with open(TAURI_CONF, encoding="utf-8") as f:
        return json.load(f)


def write_conf(conf: dict):
    with open(TAURI_CONF, "w", encoding="utf-8") as f:
        json.dump(conf, f, indent=4)
        f.write("\n")


def validate_semver(v: str) -> bool:
    parts = v.lstrip("v").split(".")
    return len(parts) == 3 and all(p.isdigit() for p in parts)


def main():
    # ── Version ──────────────────────────────────────────────────────────────
    conf = read_conf()
    current_version = conf["version"]

    if len(sys.argv) > 1:
        version = sys.argv[1].lstrip("v")
    else:
        raw = input(f"Version [{current_version}]: ").strip()
        version = raw.lstrip("v") if raw else current_version

    if not validate_semver(version):
        print(f"ERROR: '{version}' is not valid semver (e.g. 1.2.3)")
        sys.exit(1)

    if version != current_version:
        conf["version"] = version
        write_conf(conf)
        print(f"  tauri.conf.json updated: {current_version} → {version}")
    else:
        print(f"  Version unchanged: {version}")

    # ── Signing key ───────────────────────────────────────────────────────────
    if not KEY_PATH.exists():
        print(f"\nERROR: Signing key not found at {KEY_PATH}")
        print("Expected at: aloe-desktop/.tauri/aloe.key")
        sys.exit(1)

    # Tauri v2 expects the base64-encoded key string directly — it decodes internally
    private_key = KEY_PATH.read_text(encoding="utf-8").strip()

    env = os.environ.copy()
    env["TAURI_SIGNING_PRIVATE_KEY"] = private_key

    # Key password — use env var if already set, otherwise prompt (blank = no password)
    if "TAURI_SIGNING_PRIVATE_KEY_PASSWORD" not in env:
        password = getpass.getpass("Signing key password (leave blank if none): ")
        if password:
            env["TAURI_SIGNING_PRIVATE_KEY_PASSWORD"] = password

    # ── Build ─────────────────────────────────────────────────────────────────
    print(f"\nBuilding Aloe Desktop v{version} for Windows ...\n")
    result = subprocess.run(
        ["bun", "run", "tauri:build"],
        cwd=SCRIPT_DIR,
        env=env,
    )
    if result.returncode != 0:
        print("\nBuild failed — see output above.")
        sys.exit(result.returncode)

    # ── Artifacts ─────────────────────────────────────────────────────────────
    artifacts = [
        ("NSIS installer", BUNDLE_DIR / "nsis" / f"Aloe Desktop_{version}_x64-setup.exe"),
        ("MSI installer",  BUNDLE_DIR / "msi"  / f"Aloe Desktop_{version}_x64_en-US.msi"),
    ]

    print("\n─── Build artifacts ────────────────────────────────────────────")
    for label, path in artifacts:
        sig_path = Path(str(path) + ".sig")
        if path.exists():
            print(f"\n{label}:")
            print(f"  File : {path}")
            if sig_path.exists():
                sig = sig_path.read_text(encoding="utf-8").strip()
                print(f"  Sig  : {sig_path}")
                print(f"\n─── {label} signature (paste into dashboard) ───\n{sig}\n")
            else:
                print(f"  WARNING: .sig file not found at {sig_path}")
        else:
            print(f"\nWARNING: {label} not found at {path}")

    # ── Copy installer to frontend public folder ──────────────────────────────
    nsis_path = BUNDLE_DIR / "nsis" / f"Aloe Desktop_{version}_x64-setup.exe"
    if nsis_path.exists():
        FRONTEND_DIR.mkdir(parents=True, exist_ok=True)
        dest = FRONTEND_DIR / nsis_path.name
        shutil.copy2(nsis_path, dest)
        url_path = "/desktop/" + urllib.parse.quote(nsis_path.name)
        print(f"\n─── Download URL ───────────────────────────────────────────────")
        print(f"  {FRONTEND_URL}{url_path}")
    else:
        print(f"\nWARNING: NSIS installer not found, skipping frontend copy.")

    print("\nDone. Follow UPDATER.md → Publishing a release to register it in the dashboard.")


if __name__ == "__main__":
    main()
