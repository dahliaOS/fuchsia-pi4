#!/usr/bin/env python
#
# Copyright 2016 The Fuchsia Authors. All rights reserved.
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

"""Make a Rust crate for a set of generated fidl files.
"""

import ast
import optparse
import os
import sys
import zipfile

from label_to_crate import label_to_crate


def make_crate(output, gen_dir, inputs, srcroot, dep_inputs):
  name = os.path.basename(output)
  longname = label_to_crate(output)
  cargo_toml_dir = os.path.join(gen_dir, output)
  output_fn = os.path.join(cargo_toml_dir, 'Cargo.toml')
  cargo_f = file(output_fn, 'w')
  cargo_f.write('# Autogenerated by //garnet/public/lib/fidl/build/make_crate.py\n')
  cargo_f.write('[package]\n')
  cargo_f.write('name = "%s"\n' % longname)
  cargo_f.write('version = "0.1.0"\n')
  cargo_f.write('\n')
  cargo_f.write('[dependencies]\n')
  cargo_f.write('futures = "0.1"\n')
  cargo_f.write('tokio-core = "0.1.9"\n')

  lib_fn = os.path.join(cargo_toml_dir, 'src', 'lib.rs')
  src_f = file(lib_fn, 'w')
  src_f.write('// Autogenerated by //garnet/public/lib/fidl/build/make_crate.py\n')
  src_f.write('#[macro_use]\n')
  src_f.write('extern crate fidl;\n')
  src_f.write('extern crate fuchsia_zircon as zircon;\n')
  src_f.write('extern crate futures;\n')
  src_f.write('extern crate tokio_core;\n')
  src_f.write('extern crate tokio_fuchsia;\n')
  for dep in dep_inputs:
    crate = label_to_crate(dep)
    # TODO: distinguish deps from public_deps, and only do "pub" for the latter.
    src_f.write('pub extern crate %s;\n' % crate)
  src_f.write('\n')
  for i in inputs:
    rel = os.path.relpath(i, os.path.dirname(lib_fn))
    basename = os.path.splitext(os.path.basename(i))[0]
    if rel != basename + '.rs':
      src_f.write('#[path="%s"]\n' % rel)
    src_f.write('mod %s;\n' % basename)
  src_f.write('\n')
  for i in inputs:
    basename = os.path.splitext(os.path.basename(i))[0]
    src_f.write('pub use %s::*;\n' % basename)

def main():
  parser = optparse.OptionParser()

  parser.add_option('--srcroot', help='Location of source root.')
  parser.add_option('--inputs', help='List of source files for the crate.')
  parser.add_option('--dep-inputs', help='List of dependencies.')
  parser.add_option('--output', help='Path to output directory for crate.')
  parser.add_option('--gen-dir', help='Path to root of gen directory.')

  options, _ = parser.parse_args()

  inputs = []
  if (options.inputs):
    inputs = ast.literal_eval(options.inputs)
  dep_inputs = []
  if options.dep_inputs:
    dep_inputs = ast.literal_eval(options.dep_inputs)
  output = options.output
  gen_dir = options.gen_dir

  make_crate(output, gen_dir, inputs, options.srcroot, dep_inputs)

if __name__ == '__main__':
  sys.exit(main())
