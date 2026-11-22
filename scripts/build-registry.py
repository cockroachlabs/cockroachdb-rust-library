#!/usr/bin/env python3
"""Build a static sparse Cargo registry (+ landing page) for GitHub Pages.

A Cargo sparse registry is just static files served over HTTPS:

    <out>/registry/config.json          # { "dl": "...", "api": null }
    <out>/registry/<prefix>/<name>      # JSON-lines, one object per version
    <out>/crates/<name>/<name>-<ver>.crate

Consumers add to .cargo/config.toml:

    [registries.cockroachlabs]
    index = "sparse+https://cockroachlabs.github.io/cockroachdb-rust-library/registry/"

then depend on e.g. cockroachdb-sqlx = { version = "0.1", registry = "cockroachlabs" }.

This script reads `cargo metadata`, packages every publishable workspace crate
with `cargo package`, computes each .crate's sha256, and emits the index. It only
touches files under <out>; it does not publish anything.
"""
from __future__ import annotations

import argparse
import functools
import hashlib
import http.server
import json
import os
import shutil
import socketserver
import subprocess
import sys
import threading

BASE_URL = "https://cockroachlabs.github.io/cockroachdb-rust-library"
CRATES_IO = "https://github.com/rust-lang/crates.io-index"


def run(cmd: list[str], env: dict | None = None) -> str:
    return subprocess.run(
        cmd, check=True, capture_output=True, text=True, env=env
    ).stdout


class _QuietHandler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *_args):  # silence per-request logging
        pass


def serve(directory: str) -> tuple[socketserver.TCPServer, int]:
    handler = functools.partial(_QuietHandler, directory=directory)
    httpd = socketserver.ThreadingTCPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    return httpd, httpd.server_address[1]


def topo_sort(pkgs: list[dict], names: set[str]) -> list[dict]:
    """Order packages so every internal dependency precedes its dependents."""
    by_name = {p["name"]: p for p in pkgs}
    ordered: list[dict] = []
    seen: set[str] = set()

    def visit(name: str, stack: set[str]):
        if name in seen or name not in by_name:
            return
        for dep in by_name[name]["dependencies"]:
            if dep["name"] in names and dep["name"] not in stack:
                visit(dep["name"], stack | {name})
        seen.add(name)
        ordered.append(by_name[name])

    for p in pkgs:
        visit(p["name"], set())
    return ordered


def index_path(name: str) -> str:
    """Cargo's sparse-index path sharding (lowercased)."""
    n = name.lower()
    if len(n) == 1:
        return f"1/{n}"
    if len(n) == 2:
        return f"2/{n}"
    if len(n) == 3:
        return f"3/{n[0]}/{n}"
    return f"{n[0:2]}/{n[2:4]}/{n}"


def dep_entry(dep: dict) -> dict:
    """Map a `cargo metadata` dependency to a sparse-index dep object."""
    kind = dep.get("kind")  # None (normal) | "dev" | "build"
    entry = {
        "name": dep["name"],
        "req": dep["req"],
        "features": dep.get("features", []),
        "optional": dep.get("optional", False),
        "default_features": dep.get("uses_default_features", True),
        "target": dep.get("target"),
        "kind": kind if kind else "normal",
    }
    source = dep.get("source")
    if source is None:
        # Path dependency → one of our own crates → same registry.
        entry["registry"] = None
    elif source.startswith("registry+") or source.startswith("sparse+"):
        entry["registry"] = CRATES_IO
    else:
        entry["registry"] = CRATES_IO
    if dep.get("rename"):
        entry["package"] = dep["name"]
        entry["name"] = dep["rename"]
    return entry


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default="_site", help="output site directory")
    args = ap.parse_args()

    meta = json.loads(run(["cargo", "metadata", "--format-version", "1", "--no-deps"]))
    workspace_members = set(meta["workspace_default_members"]) | set(meta["workspace_members"])

    publishable = []
    for pkg in meta["packages"]:
        if pkg["id"] not in workspace_members:
            continue
        # publish == None means "allowed anywhere"; [] or False means do not publish.
        if pkg.get("publish") == [] or pkg.get("publish") is False:
            continue
        publishable.append(pkg)

    names = {p["name"] for p in publishable}
    publishable = topo_sort(publishable, names)

    out = os.path.abspath(args.out)
    reg_dir = os.path.join(out, "registry")
    crates_dir = os.path.join(out, "crates")
    os.makedirs(reg_dir, exist_ok=True)
    os.makedirs(crates_dir, exist_ok=True)

    # config.json — dl uses markers so the .crate path is explicit.
    with open(os.path.join(reg_dir, "config.json"), "w") as f:
        json.dump(
            {"dl": f"{BASE_URL}/crates/{{crate}}/{{crate}}-{{version}}.crate", "api": None},
            f,
            indent=2,
        )

    # Serve the registry as it is built so `cargo package` can resolve each
    # internal dependency from the entries already written (topological order).
    httpd, port = serve(out)
    env = dict(os.environ)
    env["CARGO_REGISTRIES_COCKROACHLABS_INDEX"] = f"sparse+http://127.0.0.1:{port}/registry/"

    target_pkg_dir = os.path.join(meta["target_directory"], "package")

    for pkg in publishable:
        name, vers = pkg["name"], pkg["version"]
        print(f"packaging {name} {vers}", file=sys.stderr)
        run(["cargo", "package", "-p", name, "--no-verify", "--allow-dirty"], env=env)

        crate_file = os.path.join(target_pkg_dir, f"{name}-{vers}.crate")
        with open(crate_file, "rb") as f:
            cksum = hashlib.sha256(f.read()).hexdigest()

        dest_dir = os.path.join(crates_dir, name)
        os.makedirs(dest_dir, exist_ok=True)
        shutil.copyfile(crate_file, os.path.join(dest_dir, f"{name}-{vers}.crate"))

        deps = [
            dep_entry(d)
            for d in pkg["dependencies"]
            if d.get("kind") != "dev"  # consumers don't need dev-deps of deps
        ]
        line = {
            "name": name,
            "vers": vers,
            "deps": deps,
            "cksum": cksum,
            "features": pkg.get("features", {}),
            "yanked": False,
            "v": 2,
        }
        if pkg.get("rust_version"):
            line["rust_version"] = pkg["rust_version"]

        ipath = os.path.join(reg_dir, index_path(name))
        os.makedirs(os.path.dirname(ipath), exist_ok=True)
        with open(ipath, "w") as f:
            f.write(json.dumps(line) + "\n")

    # Minimal landing page.
    names = "".join(f"<li><code>{p['name']}</code></li>" for p in publishable)
    with open(os.path.join(out, "index.html"), "w") as f:
        f.write(
            "<!doctype html><meta charset=utf-8>"
            "<title>CockroachDB Rust library</title>"
            "<h1>CockroachDB Rust library</h1>"
            "<p>Anonymous sparse Cargo registry. In <code>.cargo/config.toml</code>:</p>"
            f"<pre>[registries.cockroachlabs]\nindex = \"sparse+{BASE_URL}/registry/\"</pre>"
            f"<p>Crates:</p><ul>{names}</ul>"
            "<p><a href=\"doc/cockroachdb_sqlx/index.html\">API documentation</a></p>"
        )

    print(f"wrote registry for {len(publishable)} crates to {out}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
