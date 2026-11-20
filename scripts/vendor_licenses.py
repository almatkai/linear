#!/usr/bin/env python3
"""Copy published upstream license texts for the locked Cargo dependency tree."""
import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
target = ROOT / "licenses"
target.mkdir(exist_ok=True)
metadata = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--locked", "--format-version", "1"], cwd=ROOT,
))
index = []
for crate in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
    if crate["source"] is None:
        continue
    crate_dir = Path(crate["manifest_path"]).parent
    destination = target / f'{crate["name"]}-{crate["version"]}'
    license_files = {
        source for source in crate_dir.iterdir()
        if source.is_file() and source.name.upper().startswith(("LICENSE", "COPYING", "NOTICE", "COPYRIGHT", "UNLICENSE"))
    }
    if crate["license_file"]:
        license_files.add(crate_dir / crate["license_file"])
    copied = []
    for source in sorted(license_files):
        destination.mkdir(exist_ok=True)
        (destination / source.name).write_bytes(source.read_bytes())
        copied.append(f"{destination.name}/{source.name}")
    index.append({"name": crate["name"], "version": crate["version"], "license": crate["license"],
                  "licenseFiles": copied})
(target / "DEPENDENCIES.json").write_text(json.dumps(index, indent=2) + "\n", encoding="utf-8")
print(f"Recorded {len(index)} locked dependency licenses, with {sum(len(p['licenseFiles']) for p in index)} notice files.")
