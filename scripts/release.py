import pathlib
import re
import subprocess
import sys


SEMVER_RE = re.compile(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z]+(?:\.[0-9A-Za-z]+)*)?")
ROOT = pathlib.Path(__file__).resolve().parent.parent


def read(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf8")


def replace(path: str, pattern: str, repl: str, *, flags: int = re.MULTILINE, count: int = 1) -> None:
    file_path = ROOT / path
    # Read and write without newline translation: the repo is LF on every platform.
    original = file_path.read_bytes().decode("utf8").replace(chr(13), "")
    text, changed = re.subn(pattern, repl, original, count=count, flags=flags)
    if changed < 1:
        raise SystemExit(f"ERROR: pattern not found in {path}")
    file_path.write_bytes(text.encode("utf8"))
    print(f"  {path}")


def check_clean_git() -> None:
    result = subprocess.run(
        ["git", "status", "--porcelain"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    if result.stdout.strip():
        sys.stderr.write("ERROR: working tree must be clean before release\n")
        sys.stderr.write(result.stdout)
        raise SystemExit(1)


def check_release_version(version: str) -> None:
    if not SEMVER_RE.fullmatch(version):
        raise SystemExit(
            f"ERROR: invalid release version '{version}'. Expected semver like 0.2.0 or 0.2.0-beta.1"
        )


def cargo_version() -> str:
    match = re.search(r'^\s*version\s*=\s*"([^"]+)"', read("Cargo.toml"), re.MULTILINE)
    if match is None:
        raise SystemExit("ERROR: failed to read version from Cargo.toml")
    return match.group(1)


def check_version_sync() -> None:
    cargo_version()


def bump(version: str) -> None:
    check_release_version(version)
    print(f"bumping to {version}")
    replace("Cargo.toml", r'^(version\s*=\s*)"[^"]+"', f'\\g<1>"{version}"')
    print("done - run `python scripts/release.py check-version-sync` before releasing")


def main(argv: list[str]) -> int:
    if len(argv) < 2:
        raise SystemExit("usage: python scripts/release.py <command> [version]")
    command = argv[1]
    if command == "check-clean-git":
        check_clean_git()
    elif command == "check-release-version":
        check_release_version(argv[2])
    elif command == "check-version-sync":
        check_version_sync()
    elif command == "bump":
        bump(argv[2])
    else:
        raise SystemExit(f"unknown command: {command}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
