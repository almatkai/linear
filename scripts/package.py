#!/usr/bin/env python3
"""Package and verify a local Schema 3 Linear binary for a supported target."""
import argparse
import hashlib
import json
import stat
import struct
import subprocess
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TARGETS = {"x86_64-pc-windows-msvc": "Windows", "aarch64-apple-darwin": "macOS"}
SDK_REVISION = "1a11afc14693cc6463e0b95ae7d61991b4a60427"
FIXTURE_MARKER = b"TEZBAR_LINEAR_TEST_ENDPOINT"


def digest(contents):
    return hashlib.sha256(contents).hexdigest()


def git_blob(contents):
    head = f"blob {len(contents)}\0".encode()
    return hashlib.sha1(head + contents).hexdigest()


def verify_source():
    inventory = json.loads((ROOT / "SOURCE.json").read_text(encoding="utf-8"))
    actual = {p.relative_to(ROOT / "upstream").as_posix() for p in (ROOT / "upstream").rglob("*") if p.is_file()}
    assert actual == set(inventory["files"]), "Upstream inventory mismatch"
    for name, exact in inventory["files"].items():
        data = (ROOT / "upstream" / name).read_bytes()
        assert digest(data) == exact["sha256"] and git_blob(data) == exact["gitBlobSha1"], name
    data = (ROOT / "LICENSE").read_bytes()
    assert digest(data) == inventory["repositoryLicense"]["sha256"], "Upstream license changed"
    return inventory


def package(target, binary, output):
    verify_source()
    lock = (ROOT / "Cargo.lock").read_text(encoding="utf-8")
    assert f'#{SDK_REVISION}"' in lock, "SDK pin missing from Cargo.lock"
    data = binary.read_bytes()
    assert FIXTURE_MARKER not in data, "A fixture-enabled binary cannot be packaged"
    if target == "x86_64-pc-windows-msvc":
        assert data[:2] == b"MZ", "Expected a Windows PE executable"
        pe_offset, = struct.unpack_from("<I", data, 0x3C)
        assert data[pe_offset:pe_offset+6] == b"PE\0\0\x64\x86", "Expected an x64 PE executable"
        exe = "bin/tezbar-linear.exe"
    else:
        assert data[:8] == b"\xcf\xfa\xed\xfe\x0c\x00\x00\x01", "Expected an Apple Silicon Mach-O executable"
        exe = "bin/tezbar-linear"
    manifest = json.loads((ROOT / "extension.json").read_text(encoding="utf-8"))
    required = [command["name"] for command in manifest["commands"] if TARGETS[target] in command["platforms"]]
    info = json.loads(subprocess.check_output([str(binary), "--protocol-info"], timeout=15))
    assert info["id"] == manifest["id"] and info["version"] == manifest["version"] and info["protocol"] == 1
    assert info["commands"] == required, "Binary and manifest commands differ"

    members = {exe: data, "LICENSE": (ROOT / "LICENSE").read_bytes(), "NOTICE": (ROOT / "NOTICE").read_bytes()}
    upstream = ROOT / "upstream"
    for directory, prefix in [("assets", "resources"), ("metadata", "resources/upstream/metadata"), ("skills", "resources/upstream/skills")]:
        base = upstream / directory
        for file in sorted(base.rglob("*")):
            if file.is_file():
                assert not file.is_symlink(), file
                members[f"{prefix}/{file.relative_to(base).as_posix()}"] = file.read_bytes()
    for name in ["package.json", "ai.yaml"]:
        members[f"resources/upstream/{name}"] = (upstream / name).read_bytes()
    members["resources/upstream/SOURCE.json"] = (ROOT / "SOURCE.json").read_bytes()
    for file in sorted((ROOT / "licenses").rglob("*")):
        if file.is_file():
            assert not file.is_symlink(), file
            members[file.relative_to(ROOT).as_posix()] = file.read_bytes()
    assert manifest["icon"] in members
    manifest["artifacts"] = {}
    manifest["files"] = {name: digest(data) for name, data in sorted(members.items())}
    members["extension.json"] = (json.dumps(manifest, ensure_ascii=False, indent=2) + "\n").encode()

    output.mkdir(parents=True, exist_ok=True)
    base = output / f"linear-{target}"
    archive = base.with_suffix(".zip")
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as bundle:
        for name, data in sorted(members.items()):
            entry = zipfile.ZipInfo(name, date_time=(2026, 1, 1, 0, 0, 0))
            entry.create_system = 3
            entry.external_attr = (stat.S_IFREG | (0o755 if name == exe else 0o644)) << 16
            entry.compress_type = zipfile.ZIP_DEFLATED
            bundle.writestr(entry, data)

    with zipfile.ZipFile(archive) as bundle:
        assert len(bundle.namelist()) == len(members) and set(bundle.namelist()) == set(members)
        assert bundle.testzip() is None
        for name, expected in manifest["files"].items():
            assert digest(bundle.read(name)) == expected, name
        assert len(members) < 4096 and archive.stat().st_size < 128 * 1024 * 1024
    artifact = {"target": target, "sha256": digest(archive.read_bytes()), "executable": exe,
                "executableSha256": manifest["files"][exe], "memberCount": len(members),
                "size": archive.stat().st_size}
    base.with_suffix(".artifact.json").write_text(json.dumps(artifact, indent=2) + "\n")
    base.with_suffix(".files.json").write_text(json.dumps(manifest["files"], indent=2) + "\n")
    base.with_suffix(".zip.sha256").write_text(f'{artifact["sha256"]}  {archive.name}\n')
    print(json.dumps(artifact))


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--target", choices=TARGETS, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, default=ROOT / "dist")
    args = parser.parse_args()
    package(args.target, args.binary.resolve(), args.output)
