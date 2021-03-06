// Copyright 2021 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include <gtest/gtest.h>

#include "src/developer/debug/zxdb/debug_adapter/context_test.h"

namespace zxdb {

namespace {

class RequestThreadsTest : public DebugAdapterContextTest {};

}  // namespace

TEST_F(RequestThreadsTest, ListThreads) {
  InitializeDebugging();

  InjectProcess(kProcessKoid);
  // Run client to receive process started event.
  RunClient();
  InjectThread(kProcessKoid, kThreadKoid);
  // Run client to receive threads started event.
  RunClient();

  // Send Threads request from the client.
  auto response = client().send(dap::ThreadsRequest());

  // Read request and process it in server.
  context().OnStreamReadable();

  // Run client to receive threads response.
  RunClient();
  auto got = response.get();
  EXPECT_FALSE(got.error);
  EXPECT_EQ(got.response.threads.size(), 1u);
}

}  // namespace zxdb
