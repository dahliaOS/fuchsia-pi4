// Copyright 2019 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#ifndef SRC_DEVICES_TESTS_DDK_FIDL_TEST_FIDL_ASYNC_LLCPP_DRIVER_H_
#define SRC_DEVICES_TESTS_DDK_FIDL_TEST_FIDL_ASYNC_LLCPP_DRIVER_H_

#include <fuchsia/hardware/test/llcpp/fidl.h>
#include <lib/async-loop/cpp/loop.h>
#include <lib/ddk/driver.h>
#include <lib/zircon-internal/thread_annotations.h>
#include <lib/zx/event.h>
#include <lib/zx/socket.h>
#include <zircon/types.h>

#include <ddktl/device.h>
#include <ddktl/fidl.h>
#include <fbl/mutex.h>

namespace fidl {

class DdkFidlDevice;
using DeviceType = ddk::Device<DdkFidlDevice, ddk::MessageableOld>;

class DdkFidlDevice : public DeviceType, public fidl::WireServer<fuchsia_hardware_test::Device> {
 public:
  explicit DdkFidlDevice(zx_device_t* parent)
      : DeviceType(parent), loop_(&kAsyncLoopConfigNeverAttachToThread) {}

  static zx_status_t Create(void* ctx, zx_device_t* dev);
  zx_status_t Bind();

  // Device protocol implementation.
  zx_status_t DdkMessage(fidl_incoming_msg_t* msg, fidl_txn_t* txn);
  void DdkRelease();

  void GetChannel(GetChannelRequestView request, GetChannelCompleter::Sync& completer) override;
  async::Loop loop_;
};
}  // namespace fidl

#endif  // SRC_DEVICES_TESTS_DDK_FIDL_TEST_FIDL_ASYNC_LLCPP_DRIVER_H_
