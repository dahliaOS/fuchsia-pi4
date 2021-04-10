// Copyright 2019 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#ifndef SRC_DEVELOPER_DEBUG_DEBUG_AGENT_ARCH_TYPES_H_
#define SRC_DEVELOPER_DEBUG_DEBUG_AGENT_ARCH_TYPES_H_

// This file contains some types that are specific to the current CPU architecture.

namespace debug_agent {
namespace arch {

#if defined(__x86_64__)

// The type that is large enough to hold the debug breakpoint CPU instruction.
using BreakInstructionType = uint8_t;

#elif defined(__aarch64__)

using BreakInstructionType = uint32_t;

#elif defined(__riscv)

using BreakInstructionType = uint32_t;


#else
#error
#endif

}  // namespace arch
}  // namespace debug_agent

#endif  // SRC_DEVELOPER_DEBUG_DEBUG_AGENT_ARCH_TYPES_H_
