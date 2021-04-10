// Copyright 2020 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#ifndef SRC_DEVELOPER_DEBUG_DEBUG_AGENT_GENERAL_REGISTERS_H_
#define SRC_DEVELOPER_DEBUG_DEBUG_AGENT_GENERAL_REGISTERS_H_

#include <zircon/syscalls/debug.h>

#include <vector>

namespace debug_ipc {
struct Register;
}

namespace debug_agent {

// Wrapper around the general thread registers to allow them to be accessed uniformly regardless
// of the platform.
class GeneralRegisters {
 public:
  GeneralRegisters() : regs_() {}
  explicit GeneralRegisters(const zx_thread_state_general_regs& r) : regs_(r) {}

#if defined(__x86_64__)
  // Instruction pointer.
  uint64_t ip() const { return regs_.rip; }
  void set_ip(uint64_t ip) { regs_.rip = ip; }

  // Stack pointer.
  uint64_t sp() const { return regs_.rsp; }
  void set_sp(uint64_t sp) { regs_.rsp = sp; }
#elif defined(__aarch64__)
  uint64_t ip() const { return regs_.pc; }
  void set_ip(uint64_t ip) { regs_.pc = ip; }

  // Stack pointer.
  uint64_t sp() const { return regs_.sp; }
  void set_sp(uint64_t sp) { regs_.sp = sp; }
#elif defined(__riscv)
  uint64_t ip() const { return 0; }
  void set_ip(uint64_t ip) { }

  // Stack pointer.
  uint64_t sp() const { return 0; }
  void set_sp(uint64_t sp) { }
#endif

  // Appends the current general registers to the given high-level register record.
  void CopyTo(std::vector<debug_ipc::Register>& dest) const;

  zx_thread_state_general_regs& GetNativeRegisters() { return regs_; }
  const zx_thread_state_general_regs& GetNativeRegisters() const { return regs_; }

 private:
  zx_thread_state_general_regs regs_;
};

}  // namespace debug_agent

#endif  // SRC_DEVELOPER_DEBUG_DEBUG_AGENT_GENERAL_REGISTERS_H_
