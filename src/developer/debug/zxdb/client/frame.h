// Copyright 2018 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#ifndef SRC_DEVELOPER_DEBUG_ZXDB_CLIENT_FRAME_H_
#define SRC_DEVELOPER_DEBUG_ZXDB_CLIENT_FRAME_H_

#include <stdint.h>

#include <functional>
#include <optional>

#include "src/developer/debug/zxdb/client/client_object.h"
#include "src/developer/debug/zxdb/symbols/symbol_data_provider.h"
#include "src/lib/fxl/macros.h"

namespace zxdb {

class EvalContext;
class Location;
class Thread;
class Register;

// Represents one stack frame.
//
// See also FrameFingerprint (the getter for a fingerprint is on Thread).
class Frame : public ClientObject {
 public:
  explicit Frame(Session* session);
  virtual ~Frame();

  // Guaranteed non-null.
  virtual Thread* GetThread() const = 0;

  // Returns true if this is a synthetic stack frame for an inlined function.
  // Inlined functions don't have separate functions or stack pointers and
  // are generated by the debugger based on the symbols for a given location.
  virtual bool IsInline() const = 0;

  // Returns the physical stack frame associated with the current frame. This
  // is used to get the non-inlined frame an inlined frame was expanded from.
  // Non-inlined frames should return |this|.
  virtual const Frame* GetPhysicalFrame() const = 0;

  // Returns the location of the stack frame code. This will be symbolized.
  virtual const Location& GetLocation() const = 0;

  // Returns the program counter of this frame. This may be faster than
  // GetLocation().address() since it doesn't need to be symbolized.
  virtual uint64_t GetAddress() const = 0;

  // Returns the general registers that were saved with this stack frame. The
  // order is not guaranteed. The top stack frame should contain all general
  // registers which should be the current state of the CPU.
  //
  // Lower stack frames should at least contain the IP and probably SP, and if
  // any registers were found saved on the stack they will be here too.
  // Non-general registers are not saved per-frame and must be requested from
  // the thread separately.
  //
  // Inline frames will report the registers from the physical frame they're
  // associated with.
  virtual const std::vector<Register>& GetGeneralRegisters() const = 0;

  // The frame base pointer.
  //
  // This is not necessarily the "BP" register. The symbols can specify
  // an arbitrary frame base for a location and this value will reflect that.
  // If the base pointer is known-unknown, it will be reported as 0 rather than
  // nullopt (nullopt from GetBasePointer() indicates it needs an async call).
  //
  // In most cases the frame base is available synchronously (when it's in
  // a register which is the common case), but symbols can declare any DWARF
  // expression to compute the frame base.
  //
  // The synchronous version will return the base pointer if possible. If it
  // returns no value, code that can handle async calls can call the
  // asynchronous version to be notified when the value is available.
  virtual std::optional<uint64_t> GetBasePointer() const = 0;
  virtual void GetBasePointerAsync(std::function<void(uint64_t bp)> cb) = 0;

  // Returns the stack pointer at this location.
  virtual uint64_t GetStackPointer() const = 0;

  // Returns the SymbolDataProvider that can be used to evaluate symbols
  // in the context of this frame.
  virtual fxl::RefPtr<SymbolDataProvider> GetSymbolDataProvider() const = 0;

  // Returns the EvalContext that can be used to evaluate expressions in
  // the context of this frame.
  virtual fxl::RefPtr<EvalContext> GetEvalContext() const = 0;

  // Determines if the code location this frame's address corresponds to is
  // potentially ambiguous. This happens when the instruction is the beginning
  // of an inlined routine, and the address could be considered either the
  // imaginary call to the inlined routine, or its first code instruction.
  // See the Stack class declaration for more details about this case.
  virtual bool IsAmbiguousInlineLocation() const = 0;

 private:
  FXL_DISALLOW_COPY_AND_ASSIGN(Frame);
};

}  // namespace zxdb

#endif  // SRC_DEVELOPER_DEBUG_ZXDB_CLIENT_FRAME_H_
