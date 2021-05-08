// Copyright 2021 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include "src/ui/bin/root_presenter/virtual_keyboard_manager.h"

#include <fuchsia/input/virtualkeyboard/cpp/fidl.h>
#include <lib/sys/cpp/testing/component_context_provider.h>
#include <zircon/status.h>

#include <gtest/gtest.h>
#include <src/lib/testing/loop_fixture/test_loop_fixture.h>

namespace root_presenter {
namespace virtual_keyboard_manager {
namespace {

class VirtualKeyboardManagerTest : public gtest::TestLoopFixture {
 protected:
  auto* context_provider() { return &context_provider_; }

 private:
  sys::testing::ComponentContextProvider context_provider_;
};

TEST_F(VirtualKeyboardManagerTest, CtorDoesNotCrash) {
  VirtualKeyboardManager(context_provider()->context());
}

TEST_F(VirtualKeyboardManagerTest, WatchTypeAndVisibilityDoesNotCrash) {
  VirtualKeyboardManager(context_provider()->context())
      .WatchTypeAndVisibility(
          [](fuchsia::input::virtualkeyboard::TextType text_type, bool is_visible) {});
}

TEST_F(VirtualKeyboardManagerTest, NotifyDoesNotCrash) {
  VirtualKeyboardManager(context_provider()->context())
      .Notify(true, fuchsia::input::virtualkeyboard::VisibilityChangeReason::USER_INTERACTION,
              []() {});
}

}  // namespace
}  // namespace virtual_keyboard_manager
}  // namespace root_presenter
