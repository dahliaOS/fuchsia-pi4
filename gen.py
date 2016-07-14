#!/usr/bin/env python
# Copyright 2016 The Chromium Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

import argparse
import json
import os
import subprocess
import sys

# TODO: This should be configurable
package_configs = [
    "fortune",
    "ftl",
    "mojo",
    "mtl",
]

def parse_config(config, labels, binaries):
    with open(config) as f:
        c = {}
        try:
            c = json.load(f)
            labels.append(c["label"])
            for b in c["binaries"]:
                binary = {}
                binary["binary"] = b["binary"]
                binary["bootfs_path"] = b["bootfs_path"]
                binaries.append(binary)
        except Exception as e:
            print "Failed to parse config %s, error %s" % (config, str(e))

def main():
    parser = argparse.ArgumentParser(description="Generate Ninja files for Fuchsia")
    parser.add_argument("--outdir", "-o", help="output directory", default="out/Debug")
    args = parser.parse_args()

    labels = []
    binaries = []
    packages_path = os.path.abspath(os.path.dirname(__file__))
    os.chdir(packages_path)
    for c in package_configs:
        parse_config(os.path.join(packages_path, c), labels, binaries)

    with open("BUILD.gn", "w") as build_gn:
        build_gn.write("""
# NOTE: This file is auto-generated by gen.py. Do not edit by hand.

group("default") {
  testonly = true
  deps = [
    "//packages/mkbootfs",
""")
        for label in labels:
            build_gn.write("""
    "%s",""" % label)
        build_gn.write("""
  ]
}
""")
    base_path = os.path.abspath(os.path.join(packages_path, os.pardir))
    outdir_path = os.path.join(base_path, args.outdir)
    pkg_gen_dir = os.path.join(outdir_path, "gen", "packages", "mkbootfs")
    if not os.path.isdir(pkg_gen_dir):
        os.makedirs(pkg_gen_dir)
    with open(os.path.join(pkg_gen_dir, "user.bootfs.manifest"), "w") as manifest:
        for binary in binaries:
            binary_path = os.path.join(outdir_path, binary["binary"])
            manifest.write("""%s=%s
""" % (binary["bootfs_path"], binary_path))
    with open(os.path.join(pkg_gen_dir, "user.bootfs.d"), "w") as depfile:
        depfile.write("user.bootfs: gen/packages/mkbootfs/user.bootfs.manifest")
        for binary in binaries:
            depfile.write(" %s" % binary["binary"])
        depfile.write("\n")
    gn_path = os.path.join(base_path, "buildtools", "gn")
    dotfile_path = os.path.join(base_path, "packages", "dot_gn")
    subprocess.check_call(
        [gn_path, "gen", outdir_path,
         "--root=%s" % base_path,
         "--dotfile=%s" % dotfile_path,
         "--script-executable=/usr/bin/env"])
    return 0

if __name__ == "__main__":
    sys.exit(main())
