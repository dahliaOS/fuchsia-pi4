// Copyright 2017 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include "src/ui/scenic/lib/display/display_manager.h"

#include <fuchsia/ui/scenic/cpp/fidl.h>
#include <lib/syslog/cpp/macros.h>

#include "lib/fit/function.h"

namespace scenic_impl {
namespace display {

DisplayManager::DisplayManager(fit::closure display_available_cb)
    : display_available_cb_(std::move(display_available_cb)) {}

void DisplayManager::BindDefaultDisplayController(
    fidl::InterfaceHandle<fuchsia::hardware::display::Controller> controller,
    zx::channel dc_device) {
  FX_DCHECK(!default_display_controller_);
  FX_DCHECK(controller);
  FX_DCHECK(dc_device);
  default_display_controller_ = std::make_shared<fuchsia::hardware::display::ControllerSyncPtr>();
  default_display_controller_->Bind(std::move(controller));
  default_display_controller_listener_ = std::make_shared<display::DisplayControllerListener>(
      std::move(dc_device), default_display_controller_);
  default_display_controller_listener_->InitializeCallbacks(
      /*on_invalid_cb=*/nullptr, fit::bind_member(this, &DisplayManager::OnDisplaysChanged),
      fit::bind_member(this, &DisplayManager::OnClientOwnershipChange));

  // Set up callback to handle Vsync notifications, and ask controller to send these notifications.
  default_display_controller_listener_->SetOnVsyncCallback(
      fit::bind_member(this, &DisplayManager::OnVsync));
  zx_status_t vsync_status = (*default_display_controller_)->EnableVsync(true);
  if (vsync_status != ZX_OK) {
    FX_LOGS(ERROR) << "Failed to enable vsync, status: " << vsync_status;
  }
}

void DisplayManager::OnDisplaysChanged(std::vector<fuchsia::hardware::display::Info> added,
                                       std::vector<uint64_t> removed) {
  if (!default_display_) {
    FX_DCHECK(added.size());

    auto& display = added[0];
    auto& mode = display.modes[0];

    default_display_ =
        std::make_unique<Display>(display.id, mode.horizontal_resolution, mode.vertical_resolution,
                                  std::move(display.pixel_format));
    OnClientOwnershipChange(owns_display_controller_);

    if (display_available_cb_) {
      display_available_cb_();
      display_available_cb_ = nullptr;
    }
  } else {
    for (uint64_t id : removed) {
      if (default_display_->display_id() == id) {
        // TODO(fxbug.dev/23490): handle this more robustly.
        FX_CHECK(false) << "Display disconnected";
        return;
      }
    }
  }
}

void DisplayManager::OnClientOwnershipChange(bool has_ownership) {
  owns_display_controller_ = has_ownership;
  if (default_display_) {
    if (has_ownership) {
      default_display_->ownership_event().signal(fuchsia::ui::scenic::displayNotOwnedSignal,
                                                 fuchsia::ui::scenic::displayOwnedSignal);
    } else {
      default_display_->ownership_event().signal(fuchsia::ui::scenic::displayOwnedSignal,
                                                 fuchsia::ui::scenic::displayNotOwnedSignal);
    }
  }
}

void DisplayManager::OnVsync(uint64_t display_id, uint64_t timestamp, std::vector<uint64_t> images,
                             uint64_t cookie) {
  if (cookie) {
    (*default_display_controller_)->AcknowledgeVsync(cookie);
  }
  if (!default_display_) {
    return;
  }
  if (default_display_->display_id() != display_id) {
    FX_LOGS(INFO) << "DisplayManager received Vsync for unknown display: " << display_id;
    return;
  }
  default_display_->OnVsync(zx::time(timestamp), std::move(images));
}

}  // namespace display
}  // namespace scenic_impl
