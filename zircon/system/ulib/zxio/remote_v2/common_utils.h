// Copyright 2020 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#ifndef ZIRCON_SYSTEM_ULIB_ZXIO_REMOTE_V2_COMMON_UTILS_H_
#define ZIRCON_SYSTEM_ULIB_ZXIO_REMOTE_V2_COMMON_UTILS_H_

#include <fuchsia/io2/llcpp/fidl.h>
#include <lib/zxio/ops.h>

// Conversion adaptors between zxio and FIDL types.

zxio_node_protocols_t ToZxioNodeProtocols(fuchsia_io2::wire::NodeProtocols protocols);

fuchsia_io2::wire::NodeProtocols ToIo2NodeProtocols(zxio_node_protocols_t zxio_protocols);

zxio_abilities_t ToZxioAbilities(fuchsia_io2::wire::Operations abilities);

fuchsia_io2::wire::Operations ToIo2Abilities(zxio_abilities_t zxio_abilities);

#endif  // ZIRCON_SYSTEM_ULIB_ZXIO_REMOTE_V2_COMMON_UTILS_H_
