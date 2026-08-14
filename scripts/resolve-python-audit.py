"""The wheels' *runtime* dependencies, as one requirements file to audit.

Neither Python package commits a lockfile, and neither should: a library
that pinned its dependencies would pin its users'. What can still be
audited is what a user installing today actually resolves — so the extras
are read out of `pyproject.toml`, installed into a throwaway environment,
and the frozen result is what the scanner sees.

`dev` and `test` are excluded: those are a contributor's machine, and the
Cargo side already draws that line the other way. `remote` is excluded too
— it resolves to this repository's own second wheel.
"""

import subprocess
import sys
import tomllib
from pathlib import Path

SKIP = {"dev", "test", "remote", "all"}
#: This repository's own distributions. The remote wheel depends on the base
#: one, and auditing ourselves against a public database answers nothing.
OURS = {"dynamic-config-py", "dynamic-config-py-remote"}


def wanted(manifest: Path) -> list[str]:
    with manifest.open("rb") as handle:
        project = tomllib.load(handle)["project"]

    requirements = list(project.get("dependencies", []))

    for extra, entries in project.get("optional-dependencies", {}).items():
        if extra not in SKIP:
            requirements.extend(entries)

    return sorted(
        entry
        for entry in set(requirements)
        if not any(entry.startswith(name) for name in OURS)
    )


def main() -> None:
    venv, out = Path(sys.argv[1]), Path(sys.argv[2])
    requirements = [
        entry
        for manifest in sorted(Path().glob("dynamic-config-python*/pyproject.toml"))
        for entry in wanted(manifest)
    ]

    print("resolving:", ", ".join(requirements))

    subprocess.run([sys.executable, "-m", "venv", str(venv)], check=True)
    subprocess.run(
        [str(venv / "bin" / "pip"), "install", "--quiet", *requirements], check=True
    )

    frozen = subprocess.run(
        [str(venv / "bin" / "pip"), "freeze"], check=True, capture_output=True, text=True
    ).stdout

    kept = [
        line
        for line in frozen.splitlines()
        if line and not any(line.startswith(name) for name in OURS)
    ]

    out.write_text("\n".join(kept) + "\n")
    print(f"{len(kept)} packages -> {out}")


if __name__ == "__main__":
    main()
