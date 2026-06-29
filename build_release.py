#!/usr/bin/env python3
"""
Aloe Desktop release build script.
Follows the process documented in UPDATER.md.

Usage:
    python build_release.py           # interactive release build
    python build_release.py 1.2.3     # pass version directly
"""

import argparse
import getpass
import json
import os
import platform
import shutil
import subprocess
import sys
import tempfile
import urllib.parse
from pathlib import Path

SCRIPT_DIR   = Path(__file__).parent.resolve()
TAURI_CONF   = SCRIPT_DIR / "src-tauri" / "tauri.conf.json"
KEY_PATH     = SCRIPT_DIR / ".tauri" / "aloe.key"
BUNDLE_DIR   = SCRIPT_DIR / "src-tauri" / "target" / "release" / "bundle"
FRONTEND_DIR = SCRIPT_DIR.parent / "aloe-frontend" / "public" / "desktop"
FRONTEND_URL = "https://aloe.247autoarmy.in"

SUPPORTED_PLATFORMS = ("windows", "linux")
LINUX_BUNDLES = ("deb", "rpm", "appimage", "binary")
DEFAULT_LINUX_BUNDLES = ("deb", "rpm")
ARCH_LINUX_BUNDLES = ("binary",)
RELEASE_BINARY = SCRIPT_DIR / "src-tauri" / "target" / "release" / "aloe-desktop"
SYSTEM_INSTALL_DIR = Path("/opt/aloe-desktop")
SYSTEM_BINARY_PATH = SYSTEM_INSTALL_DIR / "aloe-desktop"
APPIMAGE_INSTALL_PATH = SYSTEM_INSTALL_DIR / "aloe-desktop.AppImage"
SYSTEM_LAUNCHER_PATH = Path("/usr/local/bin/aloe-desktop")
SYSTEM_ICON_PATH = Path("/usr/share/pixmaps/aloe-desktop.png")
SYSTEM_DESKTOP_PATH = Path("/usr/share/applications/aloe-desktop.desktop")


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


def host_platform() -> str:
    system = platform.system().lower()
    if system.startswith("windows"):
        return "windows"
    if system == "linux":
        return "linux"
    return system


def linux_distro_id() -> str:
    os_release = Path("/etc/os-release")
    if not os_release.exists():
        return ""

    for line in os_release.read_text(encoding="utf-8").splitlines():
        if line.startswith("ID="):
            return line.split("=", 1)[1].strip().strip('"').lower()
    return ""


def default_linux_bundles() -> list[str]:
    if linux_distro_id() == "arch":
        return list(ARCH_LINUX_BUNDLES)
    return list(DEFAULT_LINUX_BUNDLES)


def prompt_choice(prompt: str, choices: list[str], default: str) -> str:
    options = "/".join(c.upper() if c == default else c for c in choices)
    while True:
        raw = input(f"{prompt} [{options}]: ").strip().lower()
        value = raw or default
        if value in choices:
            return value
        print(f"Please choose one of: {', '.join(choices)}")


def prompt_yes_no(prompt: str, default: bool) -> bool:
    suffix = "Y/n" if default else "y/N"
    while True:
        raw = input(f"{prompt} [{suffix}]: ").strip().lower()
        if not raw:
            return default
        if raw in ("y", "yes"):
            return True
        if raw in ("n", "no"):
            return False
        print("Please answer yes or no.")


def parse_linux_bundles(value: str) -> list[str]:
    bundles = [part.strip().lower() for part in value.split(",") if part.strip()]
    invalid = [bundle for bundle in bundles if bundle not in LINUX_BUNDLES]
    if invalid:
        raise ValueError(f"unsupported Linux bundle(s): {', '.join(invalid)}")
    if "binary" in bundles and len(bundles) > 1:
        raise ValueError("binary cannot be combined with other Linux bundles")
    return list(dict.fromkeys(bundles))


def parse_args():
    parser = argparse.ArgumentParser(description="Build an Aloe Desktop release.")
    parser.add_argument("version", nargs="?", help="Release version, e.g. 1.2.3")
    parser.add_argument(
        "--platform",
        choices=SUPPORTED_PLATFORMS,
        help="Target platform. Tauri builds for the current host unless cross-compilation is configured.",
    )
    parser.add_argument(
        "--install",
        action="store_true",
        help="Install the built app after the build completes.",
    )
    parser.add_argument(
        "--no-install",
        action="store_true",
        help="Do not ask to install the built app.",
    )
    parser.add_argument(
        "--copy-frontend",
        action="store_true",
        help="Copy the Windows NSIS installer into the frontend public downloads folder.",
    )
    parser.add_argument(
        "--no-copy-frontend",
        action="store_true",
        help="Skip copying the Windows NSIS installer into the frontend public downloads folder.",
    )
    parser.add_argument(
        "--linux-bundles",
        help="Comma-separated Linux bundles to build: deb,rpm,appimage,binary. Default: binary on Arch, deb,rpm elsewhere.",
    )
    args = parser.parse_args()

    if args.install and args.no_install:
        parser.error("--install and --no-install cannot be used together")
    if args.copy_frontend and args.no_copy_frontend:
        parser.error("--copy-frontend and --no-copy-frontend cannot be used together")
    try:
        if args.linux_bundles:
            args.linux_bundles = parse_linux_bundles(args.linux_bundles)
    except ValueError as exc:
        parser.error(str(exc))

    return args


def run_command(command: list[str], env: dict | None = None) -> int:
    return subprocess.run(command, cwd=SCRIPT_DIR, env=env).returncode


def build_command(target_platform: str, linux_bundles: list[str]) -> list[str]:
    command = ["bun", "run", "tauri:build"]
    if target_platform == "linux":
        if linux_bundles == ["binary"]:
            command.append("--no-bundle")
        else:
            command.extend(["--bundles", *linux_bundles])
    return command


def find_linux_artifacts(version: str):
    patterns = {
        "Release binary": RELEASE_BINARY,
        "DEB package": BUNDLE_DIR / "deb" / "*.deb",
        "RPM package": BUNDLE_DIR / "rpm" / "*.rpm",
        "AppImage": BUNDLE_DIR / "appimage" / "*.AppImage",
    }
    artifacts = []
    for label, pattern in patterns.items():
        if "*" not in pattern.name:
            if pattern.exists():
                artifacts.append((label, pattern))
            continue

        matches = sorted(pattern.parent.glob(pattern.name), key=lambda p: p.stat().st_mtime, reverse=True)
        if version:
            version_matches = [p for p in matches if version in p.name]
            matches = version_matches or matches
        artifacts.extend((label, path) for path in matches)
    return artifacts


def windows_artifacts(version: str):
    return [
        ("NSIS installer", BUNDLE_DIR / "nsis" / f"Aloe Desktop_{version}_x64-setup.exe"),
        ("MSI installer",  BUNDLE_DIR / "msi"  / f"Aloe Desktop_{version}_x64_en-US.msi"),
    ]


def print_artifacts(artifacts):
    print("\n─── Build artifacts ────────────────────────────────────────────")
    if not artifacts:
        print("\nWARNING: No build artifacts found.")
        return

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
            print(f"\nWARNING: {label} not found at {path}")


def copy_windows_installer_to_frontend(version: str):
    nsis_path = BUNDLE_DIR / "nsis" / f"Aloe Desktop_{version}_x64-setup.exe"
    if not nsis_path.exists():
        print("\nWARNING: NSIS installer not found, skipping frontend copy.")
        return

    FRONTEND_DIR.mkdir(parents=True, exist_ok=True)
    dest = FRONTEND_DIR / nsis_path.name
    shutil.copy2(nsis_path, dest)
    url_path = "/desktop/" + urllib.parse.quote(nsis_path.name)
    print("\n─── Download URL ───────────────────────────────────────────────")
    print(f"  {FRONTEND_URL}{url_path}")


def install_windows_artifact(version: str):
    nsis_path = BUNDLE_DIR / "nsis" / f"Aloe Desktop_{version}_x64-setup.exe"
    msi_path = BUNDLE_DIR / "msi" / f"Aloe Desktop_{version}_x64_en-US.msi"
    installer = nsis_path if nsis_path.exists() else msi_path

    if not installer.exists():
        print("\nWARNING: No Windows installer found to install.")
        return

    print(f"\nStarting installer: {installer}")
    os.startfile(installer)  # type: ignore[attr-defined]


def install_with_sudo(source: Path, destination: Path, mode: str):
    command = ["sudo", "install", f"-m{mode}", "-D", str(source), str(destination)]
    result = subprocess.run(command, cwd=SCRIPT_DIR)
    if result.returncode != 0:
        print("\nInstall failed — see output above.")
        sys.exit(result.returncode)


def install_arch_appimage(appimage: Path):
    icon = SCRIPT_DIR / "src-tauri" / "icons" / "icon.png"
    if not icon.exists():
        icon = SCRIPT_DIR / "src-tauri" / "icons" / "128x128.png"

    with tempfile.TemporaryDirectory(prefix="aloe-desktop-install-") as tmp:
        tmp_dir = Path(tmp)
        launcher = tmp_dir / "aloe-desktop"
        desktop = tmp_dir / "aloe-desktop.desktop"

        launcher.write_text(
            f"""#!/usr/bin/env sh
exec "{APPIMAGE_INSTALL_PATH}" "$@"
""",
            encoding="utf-8",
        )
        desktop.write_text(
            f"""[Desktop Entry]
Type=Application
Name=Aloe Desktop
Exec={SYSTEM_LAUNCHER_PATH}
Icon=aloe-desktop
Terminal=false
Categories=Utility;
StartupWMClass=Aloe Desktop
""",
            encoding="utf-8",
        )

        print(f"\nInstalling AppImage system-wide for Arch: {appimage}")
        install_with_sudo(appimage, APPIMAGE_INSTALL_PATH, "755")
        install_with_sudo(launcher, SYSTEM_LAUNCHER_PATH, "755")
        if icon.exists():
            install_with_sudo(icon, SYSTEM_ICON_PATH, "644")
        else:
            print("\nWARNING: No icon found, skipping system icon install.")
        install_with_sudo(desktop, SYSTEM_DESKTOP_PATH, "644")

    print(f"\nInstalled Aloe Desktop:")
    print(f"  AppImage : {APPIMAGE_INSTALL_PATH}")
    print(f"  Launcher : {SYSTEM_LAUNCHER_PATH}")
    print(f"  Desktop  : {SYSTEM_DESKTOP_PATH}")


def install_arch_binary(binary: Path):
    icon = SCRIPT_DIR / "src-tauri" / "icons" / "icon.png"
    if not icon.exists():
        icon = SCRIPT_DIR / "src-tauri" / "icons" / "128x128.png"

    with tempfile.TemporaryDirectory(prefix="aloe-desktop-install-") as tmp:
        tmp_dir = Path(tmp)
        launcher = tmp_dir / "aloe-desktop"
        desktop = tmp_dir / "aloe-desktop.desktop"

        launcher.write_text(
            f"""#!/usr/bin/env sh
exec "{SYSTEM_BINARY_PATH}" "$@"
""",
            encoding="utf-8",
        )
        desktop.write_text(
            f"""[Desktop Entry]
Type=Application
Name=Aloe Desktop
Exec={SYSTEM_LAUNCHER_PATH}
Icon=aloe-desktop
Terminal=false
Categories=Utility;
StartupWMClass=Aloe Desktop
""",
            encoding="utf-8",
        )

        print(f"\nInstalling release binary system-wide for Arch: {binary}")
        install_with_sudo(binary, SYSTEM_BINARY_PATH, "755")
        install_with_sudo(launcher, SYSTEM_LAUNCHER_PATH, "755")
        if icon.exists():
            install_with_sudo(icon, SYSTEM_ICON_PATH, "644")
        else:
            print("\nWARNING: No icon found, skipping system icon install.")
        install_with_sudo(desktop, SYSTEM_DESKTOP_PATH, "644")

    print(f"\nInstalled Aloe Desktop:")
    print(f"  Binary   : {SYSTEM_BINARY_PATH}")
    print(f"  Launcher : {SYSTEM_LAUNCHER_PATH}")
    print(f"  Desktop  : {SYSTEM_DESKTOP_PATH}")


def install_linux_artifact(version: str, linux_bundles: list[str]):
    binaries = [path for label, path in find_linux_artifacts(version) if label == "Release binary"]
    debs = [path for _, path in find_linux_artifacts(version) if path.suffix == ".deb"]
    rpms = [path for _, path in find_linux_artifacts(version) if path.suffix == ".rpm"]
    appimages = [path for _, path in find_linux_artifacts(version) if path.suffix == ".AppImage"]

    if linux_distro_id() == "arch" and "binary" in linux_bundles and binaries:
        install_arch_binary(binaries[0])
        return
    if linux_distro_id() == "arch" and "appimage" in linux_bundles and appimages:
        install_arch_appimage(appimages[0])
        return
    if debs and shutil.which("apt"):
        package = debs[0]
        command = ["sudo", "apt", "install", "-y", str(package)]
    elif debs and shutil.which("dpkg"):
        package = debs[0]
        command = ["sudo", "dpkg", "-i", str(package)]
    elif rpms and shutil.which("dnf"):
        package = rpms[0]
        command = ["sudo", "dnf", "install", "-y", str(package)]
    elif rpms and shutil.which("rpm"):
        package = rpms[0]
        command = ["sudo", "rpm", "-Uvh", str(package)]
    elif appimages:
        package = appimages[0]
        app_dir = Path.home() / ".local" / "bin"
        app_dir.mkdir(parents=True, exist_ok=True)
        dest = app_dir / "aloe-desktop.AppImage"
        shutil.copy2(package, dest)
        dest.chmod(dest.stat().st_mode | 0o111)
        print(f"\nInstalled AppImage launcher at {dest}")
        print("Make sure ~/.local/bin is on your PATH.")
        return
    else:
        print("\nWARNING: No installable Linux artifact found.")
        return

    print(f"\nInstalling {package} ...")
    result = subprocess.run(command, cwd=SCRIPT_DIR)
    if result.returncode != 0:
        print("\nInstall failed — see output above.")
        sys.exit(result.returncode)


def main():
    args = parse_args()
    interactive = sys.stdin.isatty()
    host = host_platform()

    if host not in SUPPORTED_PLATFORMS:
        print(f"ERROR: unsupported host platform '{host}'. Supported: {', '.join(SUPPORTED_PLATFORMS)}")
        sys.exit(1)

    if args.platform:
        target_platform = args.platform
    elif interactive:
        target_platform = prompt_choice("Build platform", list(SUPPORTED_PLATFORMS), host)
    else:
        target_platform = host

    if target_platform != host:
        print(
            f"ERROR: requested {target_platform}, but this script is running on {host}. "
            "Tauri release builds are host-native unless cross-compilation is configured."
        )
        sys.exit(1)

    # ── Version ──────────────────────────────────────────────────────────────
    conf = read_conf()
    current_version = conf["version"]

    if args.version:
        version = args.version.lstrip("v")
    elif interactive:
        raw = input(f"Version [{current_version}]: ").strip()
        version = raw.lstrip("v") if raw else current_version
    else:
        version = current_version

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
    if args.install:
        should_install = True
    elif args.no_install:
        should_install = False
    elif interactive:
        should_install = prompt_yes_no("Install the app after building?", False)
    else:
        should_install = False

    if target_platform == "windows":
        copy_frontend = args.copy_frontend or (not args.no_copy_frontend and interactive and prompt_yes_no(
            "Copy Windows installer to frontend downloads folder?", True
        ))
    else:
        copy_frontend = False
        if not args.linux_bundles:
            args.linux_bundles = default_linux_bundles()

    if target_platform == "linux" and interactive:
        current = ",".join(args.linux_bundles)
        raw = input(f"Linux bundles [{current}]: ").strip()
        if raw:
            try:
                args.linux_bundles = parse_linux_bundles(raw)
            except ValueError as exc:
                print(f"ERROR: {exc}")
                sys.exit(1)

    if target_platform == "linux" and not args.linux_bundles:
        print("ERROR: at least one Linux bundle must be selected.")
        sys.exit(1)

    if (
        target_platform == "linux"
        and linux_distro_id() == "arch"
        and should_install
        and not {"binary", "appimage"}.intersection(args.linux_bundles)
    ):
        print("ERROR: Arch system install requires the binary or appimage bundle.")
        print("Use: --linux-bundles binary --install")
        sys.exit(1)

    if target_platform == "linux" and "appimage" in args.linux_bundles:
        # linuxdeploy's bundled strip can fail on newer distro libraries with
        # .relr.dyn sections, so avoid stripping when AppImage is requested.
        env.setdefault("NO_STRIP", "1")

    print(f"\nBuilding Aloe Desktop v{version} for {target_platform.title()} ...\n")
    returncode = run_command(build_command(target_platform, args.linux_bundles), env=env)
    if returncode != 0:
        print("\nBuild failed — see output above.")
        sys.exit(returncode)

    # ── Artifacts ─────────────────────────────────────────────────────────────
    artifacts = windows_artifacts(version) if target_platform == "windows" else find_linux_artifacts(version)
    print_artifacts(artifacts)

    # ── Copy installer to frontend public folder ──────────────────────────────
    if copy_frontend:
        copy_windows_installer_to_frontend(version)

    if should_install:
        if target_platform == "windows":
            install_windows_artifact(version)
        else:
            install_linux_artifact(version, args.linux_bundles)

    print("\nDone. Follow UPDATER.md → Publishing a release to register it in the dashboard.")


if __name__ == "__main__":
    main()
