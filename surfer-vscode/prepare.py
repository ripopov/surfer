#!/usr/bin/env python3

import re
import tomllib
import json
import subprocess

with open("../Cargo.toml", "rb") as f:
    data = tomllib.load(f)
    print(data["workspace"]["package"]["version"])
    surfer_version_raw = data['workspace']["package"]["version"]

surfer_git_rev_list_raw = subprocess.check_output([
    "git", "rev-list", "HEAD"
], encoding="utf-8", cwd="..").split()


surfer_version = re.match(r"^(\d+)\.(\d+).(\d+)?(-dev)?$", surfer_version_raw)
surfer_major  = int(surfer_version[1])
surfer_minor  = int(surfer_version[2])
surfer_patch = int(surfer_version[3])
# If -dev is present, decrement surfer_minor
if surfer_version.groups()[3]:
    print(f"Surfer patch is {surfer_patch} and -dev is present. Decrementing patch")
    surfer_minor = surfer_minor- 1

version = f"{surfer_major}.{surfer_minor}.1{surfer_patch:02}{len(surfer_git_rev_list_raw):04}"
print(f"version {version}")

with open("extension/package-in.json", "rt") as f:
    package_json = json.load(f)
package_json["version"] = version
with open("extension/package.json", "wt") as f:
    json.dump(package_json, f, indent=2)
