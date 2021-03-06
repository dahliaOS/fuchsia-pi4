// Copyright 2021 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include <lib/fit/promise.h>
#include <lib/fit/single_threaded_executor.h>
#include <lib/sys/cpp/component_context.h>
#include <lib/sys/cpp/testing/test_with_environment.h>

#include <gtest/gtest.h>

#include "src/ui/lib/display/get_hardware_display_controller.h"
#include "src/ui/lib/display/hardware_display_controller_provider_impl.h"

namespace ui_display {
namespace test {

struct fake_context : fit::context {
  fit::executor* executor() const override { return nullptr; }
  fit::suspended_task suspend_task() override { return fit::suspended_task(); }
};

class GetHardwareDisplayControllerInjectServicesTest : public sys::testing::TestWithEnvironment {};

// Tests the code path when the service is injected through .cmx file.
TEST_F(GetHardwareDisplayControllerInjectServicesTest, WithInjectedService) {
  auto promise = GetHardwareDisplayController();
  fake_context context;
  EXPECT_FALSE(promise(context).is_error());
}

}  // namespace test
}  // namespace ui_display
