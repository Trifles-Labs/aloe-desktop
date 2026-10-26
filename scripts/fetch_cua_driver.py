#!/usr/bin/env python3
"""
Fetch the pinned Cua Driver release into src-tauri/resources/cua-driver/, where tauri.conf.json
bundles it with the app (see src-tauri/src/cua.rs). Run once before `bun tauri dev` or a build;
build_release.py runs it for you.

Usage:
    python scripts/fetch_cua_driver.py           # host platform and architecture
    python scripts/fetch_cua_driver.py --force   # re-download even if already present

To move to a newer driver, update CUA_DRIVER_VERSION and the hashes from that release's
checksums.txt (github.com/trycua/cua/releases/tag/cua-driver-rs-v<version>).
"""

import argparse
import hashlib
import io
import platform
import shutil
import sys
import tarfile
import urllib.request
import zipfile
from pathlib import Path

CUA_DRIVER_VERSION = "0.38.0"
RELEASE_URL = f"https://github.com/trycua/cua/releases/download/cua-driver-rs-v{CUA_DRIVER_VERSION}"

# (system, machine) -> (archive name, sha256 from the release's checksums.txt)
ARCHIVES = {
    ("windows", "x86_64"): (f"cua-driver-rs-{CUA_DRIVER_VERSION}-windows-x86_64-binary.zip", "5edaa74d0afa1b55a9668dbc4f32736507bd2bd4d75b18ca1dc8881bb12402b5"),
    ("windows", "arm64"): (f"cua-driver-rs-{CUA_DRIVER_VERSION}-windows-arm64-binary.zip", "d25c8ebfffb34e4c095ea170baf47b87c35dc0d1116f42e46b39df2bcbff8700"),
    ("linux", "x86_64"): (f"cua-driver-rs-{CUA_DRIVER_VERSION}-linux-x86_64-binary.tar.gz", "31ab9709dd91ce73677e49b9f3e75c6f0d3e4d9625c286c9e58262bfbea698dd"),
    ("linux", "arm64"): (f"cua-driver-rs-{CUA_DRIVER_VERSION}-linux-arm64-binary.tar.gz", "80c9ae66bc7322052dd426c6133c96086b33894bf06c972bca8215384400b33d"),
    # Universal: the release workflow builds Aloe for universal-apple-darwin.
    ("darwin", "x86_64"): (f"cua-driver-rs-{CUA_DRIVER_VERSION}-darwin-universal-binary.tar.gz", "2bb8dc4a7e69185ffd04c4a9c0eab590bd2c54b72c2c1be13ab675be4f487732"),
    ("darwin", "arm64"): (f"cua-driver-rs-{CUA_DRIVER_VERSION}-darwin-universal-binary.tar.gz", "2bb8dc4a7e69185ffd04c4a9c0eab590bd2c54b72c2c1be13ab675be4f487732"),
}

# The executables the MCP server needs. The archives also carry an SDK library, a Node runtime
# and a C header for in-process embedding, which Aloe does not use.
KEEP_PREFIXES = ("cua-driver", "cua-cursor-theme")

SCRIPT_DIR = Path(__file__).parent.resolve()
DEST_DIR = SCRIPT_DIR.parent / "src-tauri" / "resources" / "cua-driver"
VERSION_MARKER = DEST_DIR / ".version"


def host_key() -> tuple[str, str]:
    system = platform.system().lower()
    machine = platform.machine().lower()
    machine = {"amd64": "x86_64", "x64": "x86_64", "aarch64": "arm64"}.get(machine, machine)
    return system, machine


def wanted(name: str) -> bool:
    base = Path(name).name
    return "/" not in name.strip("/") and base.startswith(KEEP_PREFIXES) and not base.endswith((".h", ".node"))


def extract(archive_name: str, data: bytes) -> list[str]:
    written = []
    if archive_name.endswith(".zip"):
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            for info in archive.infolist():
                if info.is_dir() or not wanted(info.filename):
                    continue
                target = DEST_DIR / Path(info.filename).name
                target.write_bytes(archive.read(info))
                written.append(target.name)
    else:
        with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
            for member in archive.getmembers():
                if not member.isfile() or not wanted(member.name):
                    continue
                target = DEST_DIR / Path(member.name).name
                source = archive.extractfile(member)
                if source is None:
                    continue
                target.write_bytes(source.read())
                target.chmod(0o755)
                written.append(target.name)
    return written


def main() -> int:
    parser = argparse.ArgumentParser(description="Fetch the pinned Cua Driver for bundling.")
    parser.add_argument("--force", action="store_true", help="re-download even if the pinned version is present")
    args = parser.parse_args()

    key = host_key()
    if key not in ARCHIVES:
        print(f"No Cua Driver build is pinned for {key[0]}/{key[1]}.", file=sys.stderr)
        return 1
    archive_name, expected_sha = ARCHIVES[key]

    if not args.force and VERSION_MARKER.exists() and VERSION_MARKER.read_text().strip() == archive_name:
        print(f"Cua Driver {CUA_DRIVER_VERSION} already present in {DEST_DIR}")
        return 0

    url = f"{RELEASE_URL}/{archive_name}"
    print(f"Downloading {url}")
    with urllib.request.urlopen(url) as response:
        data = response.read()
    actual_sha = hashlib.sha256(data).hexdigest()
    if actual_sha != expected_sha:
        print(f"Checksum mismatch for {archive_name}: expected {expected_sha}, got {actual_sha}", file=sys.stderr)
        return 1

    if DEST_DIR.exists():
        shutil.rmtree(DEST_DIR)
    DEST_DIR.mkdir(parents=True)
    written = extract(archive_name, data)
    if not any(name in ("cua-driver", "cua-driver.exe") for name in written):
        print(f"{archive_name} did not contain the cua-driver executable", file=sys.stderr)
        return 1
    VERSION_MARKER.write_text(archive_name + "\n")
    print(f"Cua Driver {CUA_DRIVER_VERSION} -> {DEST_DIR}: {', '.join(sorted(written))}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
