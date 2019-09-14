// Copyright 2017 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#ifndef SRC_MODULAR_LIB_TESTING_FAKE_AGENT_RUNNER_STORAGE_H_
#define SRC_MODULAR_LIB_TESTING_FAKE_AGENT_RUNNER_STORAGE_H_

#include <functional>
#include <string>

#include <src/lib/fxl/macros.h>

#include "src/modular/bin/sessionmgr/agent_runner/agent_runner_storage.h"

namespace modular {
namespace testing {

class FakeAgentRunnerStorage : public AgentRunnerStorage {
 public:
  FakeAgentRunnerStorage() = default;

  // |AgentRunnerStorage|
  void Initialize(NotificationDelegate* /*delegate*/, fit::function<void()> done) override {
    done();
  }

  FXL_DISALLOW_COPY_AND_ASSIGN(FakeAgentRunnerStorage);
};

}  // namespace testing
}  // namespace modular

#endif  // SRC_MODULAR_LIB_TESTING_FAKE_AGENT_RUNNER_STORAGE_H_
