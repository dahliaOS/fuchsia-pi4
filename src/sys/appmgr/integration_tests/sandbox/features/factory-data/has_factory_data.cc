// Copyright 2020 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include <fuchsia/io/llcpp/fidl.h>

#include "src/sys/appmgr/integration_tests/sandbox/namespace_test.h"

namespace fio = fuchsia_io;

TEST_F(NamespaceTest, HasFactoryFolder) {
  ExpectExists("/factory");
  ExpectPathSupportsStrictRights("/factory", fio::wire::kOpenRightReadable);
}
