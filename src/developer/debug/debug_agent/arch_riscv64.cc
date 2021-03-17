// Copyright 2018 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include <lib/syslog/cpp/macros.h>
#include <zircon/status.h>
#include <zircon/syscalls/exception.h>

#include "src/developer/debug/debug_agent/arch.h"
#include "src/developer/debug/debug_agent/arch_types.h"
#include "src/developer/debug/debug_agent/debugged_thread.h"
#include "src/developer/debug/ipc/decode_exception.h"
#include "src/developer/debug/ipc/register_desc.h"
#include "src/developer/debug/shared/logging/logging.h"
#include "src/developer/debug/shared/zx_status.h"
#include "src/lib/fxl/strings/string_printf.h"

namespace debug_agent {
namespace arch {

using debug_ipc::Register;
using debug_ipc::RegisterID;

const BreakInstructionType kBreakInstruction = 0x00000000;

const int64_t kExceptionOffsetForSoftwareBreakpoint = 0;

::debug_ipc::Arch GetCurrentArch() { return ::debug_ipc::Arch::kRiscv64; }

void SaveGeneralRegs(const zx_thread_state_general_regs& input,
                     std::vector<debug_ipc::Register>& out) {
}

zx_status_t ReadRegisters(const zx::thread& thread, const debug_ipc::RegisterCategory& cat,
                          std::vector<debug_ipc::Register>& out) {
  return ZX_ERR_INVALID_ARGS;
}

zx_status_t WriteRegisters(zx::thread& thread, const debug_ipc::RegisterCategory& category,
                           const std::vector<debug_ipc::Register>& registers) {
  return ZX_ERR_INVALID_ARGS;
}

zx_status_t WriteGeneralRegisters(const std::vector<Register>& updates,
                                  zx_thread_state_general_regs_t* regs) {
  return ZX_ERR_INVALID_ARGS;
}

zx_status_t WriteVectorRegisters(const std::vector<Register>& updates,
                                 zx_thread_state_vector_regs_t* regs) {
  return ZX_ERR_INVALID_ARGS;
}

zx_status_t WriteDebugRegisters(const std::vector<Register>& updates,
                                zx_thread_state_debug_regs_t* regs) {
  return ZX_ERR_INVALID_ARGS;
}

debug_ipc::ExceptionType DecodeExceptionType(const zx::thread& thread, uint32_t exception_type) {
  return debug_ipc::ExceptionType::kUnknown;
}

debug_ipc::ExceptionRecord FillExceptionRecord(const zx_exception_report_t& in) {
  debug_ipc::ExceptionRecord record;

  record.valid = false;

  return record;
}

uint64_t NextInstructionForSoftwareExceptionAddress(uint64_t exception_addr) {
  return exception_addr + 4;
}

bool IsBreakpointInstruction(BreakInstructionType instruction) {
  return false;
}

uint64_t BreakpointInstructionForHardwareExceptionAddress(uint64_t exception_addr) {
  return exception_addr;
}

}  // namespace arch
}  // namespace debug_agent
