// Copyright 2019 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#ifndef SRC_DEVELOPER_DEBUG_ZXDB_SYMBOLS_DWARF_LOCATION_H_
#define SRC_DEVELOPER_DEBUG_ZXDB_SYMBOLS_DWARF_LOCATION_H_

#include "src/developer/debug/zxdb/symbols/arch.h"
#include "src/developer/debug/zxdb/symbols/variable_location.h"
#include "src/lib/containers/cpp/array_view.h"

namespace llvm {
class DWARFUnit;
class DWARFFormValue;
}  // namespace llvm

namespace zxdb {

// Decodes the variable location contained in the given form value. It's assumed the form value
// contains either a block, an ExprLoc, or an offset into the .debug_loc section.
//
// The source is the symbol that created this location entry, it will be passed to the
// VariableLocation.
//
// On error this will return an empty VariableLocation.
VariableLocation DecodeVariableLocation(const llvm::DWARFUnit* unit,
                                        const llvm::DWARFFormValue& form,
                                        const UncachedLazySymbol& source);

// Low-level decode for a variable location description. The data should start at the beginning
// of the location list to parse, and cover as much data as the location list could possibly
// cover (normally the end of the .debug_loc section).
//
// The source is the symbol that created this location entry, it will be passed to the
// VariableLocation.
//
// On error this will return an empty VariableLocation.
VariableLocation DecodeLocationList(TargetPointer unit_base_addr,
                                    containers::array_view<uint8_t> data,
                                    const UncachedLazySymbol& source);

}  // namespace zxdb

#endif  // SRC_DEVELOPER_DEBUG_ZXDB_SYMBOLS_DWARF_LOCATION_H_
