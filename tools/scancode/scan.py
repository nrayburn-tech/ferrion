"""Runs ScanCode over the repository and checks the results.

Usage: python3 tools/scancode/scan.py

Needs Python 3.13, which requirements.txt is locked for, and libmagic (on macOS,
`brew install libmagic`). ScanCode is installed from requirements.txt into target/scancode/venv,
and reinstalled whenever requirements.txt changes.

Only the files Git would track are scanned: committed files, and untracked files that aren't
ignored. They are copied to target/scancode/files first, so build output and local tool files are
never scanned.

To change the dependencies, edit requirements.in, then regenerate requirements.txt with pip-tools,
from this directory:

    pip-compile --generate-hashes --allow-unsafe --strip-extras \\
        --output-file requirements.txt requirements.in
"""

import glob
import os
import shutil
import subprocess
import sys
import venv
from pathlib import Path

import check

PYTHON_VERSION = (3, 13)
ROOT = Path(__file__).resolve().parents[2]
REQUIREMENTS = ROOT / "tools" / "scancode" / "requirements.txt"
OUT = ROOT / "target" / "scancode"


# Where libmagic and its database usually are, as (library, database) patterns from the same
# installation: Homebrew on Apple silicon and Intel, then Debian and Ubuntu.
LIBMAGIC_LOCATIONS = [
    ("/opt/homebrew/lib/libmagic.dylib", "/opt/homebrew/share/misc/magic.mgc"),
    ("/usr/local/lib/libmagic.dylib", "/usr/local/share/misc/magic.mgc"),
    ("/usr/lib/*-linux-gnu/libmagic.so.1", "/usr/lib/file/magic.mgc"),
]


def main():
    if sys.version_info[:2] != PYTHON_VERSION:
        version = ".".join(map(str, PYTHON_VERSION))
        sys.exit(f"scan.py: needs Python {version}, but this is {sys.version.split()[0]}")
    scancode = install()
    files = stage()
    report = OUT / "scancode.json"
    subprocess.run(
        [
            scancode,
            "--license",
            "--copyright",
            "--strip-root",
            "--processes",
            str(os.cpu_count() or 1),
            "--quiet",
            "--json-pp",
            report,
            files,
        ],
        check=True,
        env=libmagic_env(),
    )
    return check.main(report)


def libmagic_env():
    """Tells ScanCode where libmagic and its database are, unless the environment already does."""
    env = dict(os.environ)
    if "TYPECODE_LIBMAGIC_PATH" in env and "TYPECODE_LIBMAGIC_DB_PATH" in env:
        return env
    for library, database in LIBMAGIC_LOCATIONS:
        libraries = glob.glob(library)
        if libraries and os.path.isfile(database):
            env["TYPECODE_LIBMAGIC_PATH"] = libraries[0]
            env["TYPECODE_LIBMAGIC_DB_PATH"] = database
            return env
    sys.exit(
        "scan.py: libmagic not found; install it (on macOS, `brew install libmagic`), or set "
        "TYPECODE_LIBMAGIC_PATH and TYPECODE_LIBMAGIC_DB_PATH"
    )


def install():
    """Creates the virtual environment if requirements.txt changed, and returns ScanCode's path."""
    env = OUT / "venv"
    bin_dir = env / ("Scripts" if os.name == "nt" else "bin")
    installed = env / "requirements.txt"
    if not installed.exists() or installed.read_bytes() != REQUIREMENTS.read_bytes():
        shutil.rmtree(env, ignore_errors=True)
        venv.create(env, with_pip=True)
        subprocess.run(
            [
                bin_dir / "python",
                "-m",
                "pip",
                "install",
                "--quiet",
                "--require-hashes",
                "--no-deps",
                "--requirement",
                REQUIREMENTS,
            ],
            check=True,
        )
        shutil.copyfile(REQUIREMENTS, installed)
    return bin_dir / "scancode"


def stage():
    """Copies the files Git would track into a fresh directory, and returns its path."""
    files = OUT / "files"
    shutil.rmtree(files, ignore_errors=True)
    listing = subprocess.run(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"],
        cwd=ROOT,
        check=True,
        capture_output=True,
    ).stdout
    for name in listing.decode().split("\0"):
        source = ROOT / name
        # Files deleted from the working tree are still listed until the deletion is committed.
        if name and source.is_file():
            target = files / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, target)
    return files


if __name__ == "__main__":
    sys.exit(main())
