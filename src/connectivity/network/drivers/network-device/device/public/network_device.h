// Copyright 2020 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#ifndef SRC_CONNECTIVITY_NETWORK_DRIVERS_NETWORK_DEVICE_DEVICE_PUBLIC_NETWORK_DEVICE_H_
#define SRC_CONNECTIVITY_NETWORK_DRIVERS_NETWORK_DEVICE_DEVICE_PUBLIC_NETWORK_DEVICE_H_

#include <fuchsia/hardware/network/device/cpp/banjo.h>
#include <fuchsia/hardware/network/llcpp/fidl.h>
#include <lib/async/dispatcher.h>
#include <lib/fit/function.h>

#include <memory>

#include <fbl/alloc_checker.h>

namespace network {

namespace netdev = fuchsia_hardware_network;

class NetworkDeviceInterface {
 public:
  virtual ~NetworkDeviceInterface() = default;
  // Creates a new NetworkDeviceInterface that will bind to the provided parent.
  // The dispatcher is only used for slow path operations, NetworkDevice will create and manage its
  // own threads for fast path operations.
  // The parent_name argument is only used for diagnostic purposes.
  static zx::status<std::unique_ptr<NetworkDeviceInterface>> Create(
      async_dispatcher_t* dispatcher, ddk::NetworkDeviceImplProtocolClient parent,
      const char* parent_name);

  // Tears down the NetworkDeviceInterface.
  // A NetworkDeviceInterface must not be destroyed until the callback provided to teardown is
  // triggered, doing so may cause an assertion error. Immediately destroying a NetworkDevice that
  // never succeeded Init is allowed.
  virtual void Teardown(fit::callback<void()>) = 0;

  // Binds the request channel req to this NetworkDeviceInterface. Requests will be handled on the
  // dispatcher given to the device on creation.
  virtual zx_status_t Bind(fidl::ServerEnd<netdev::Device> req) = 0;

  // Binds the request channel req to PORT 0 of this device. Requests will be handled on the
  // dispatcher given to the device on creation.
  //
  // TODO(http://fxbug.dev/64310): Remove this method after multi-port is supported on FIDL.
  virtual zx_status_t BindMac(fidl::ServerEnd<netdev::MacAddressing> req) = 0;

 protected:
  NetworkDeviceInterface() = default;
};

}  // namespace network

#endif  // SRC_CONNECTIVITY_NETWORK_DRIVERS_NETWORK_DEVICE_DEVICE_PUBLIC_NETWORK_DEVICE_H_
