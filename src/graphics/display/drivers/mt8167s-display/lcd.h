// Copyright 2019 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#ifndef SRC_GRAPHICS_DISPLAY_DRIVERS_MT8167S_DISPLAY_LCD_H_
#define SRC_GRAPHICS_DISPLAY_DRIVERS_MT8167S_DISPLAY_LCD_H_

#include <fuchsia/hardware/dsiimpl/cpp/banjo.h>
#include <fuchsia/hardware/gpio/cpp/banjo.h>
#include <unistd.h>
#include <zircon/compiler.h>

#include <hwreg/mmio.h>

namespace mt8167s_display {

class Lcd {
 public:
  Lcd(const ddk::DsiImplProtocolClient* dsi, const ddk::GpioProtocolClient* gpio,
      uint32_t panel_type)
      : dsiimpl_(*dsi), gpio_(*gpio), panel_type_(panel_type) {}

  zx_status_t Init();
  zx_status_t Enable();
  zx_status_t Disable();
  void PowerOn();
  void PowerOff();

 private:
  zx_status_t LoadInitTable(const uint8_t* buffer, size_t size);
  zx_status_t GetDisplayId(uint16_t& id);

  const ddk::DsiImplProtocolClient dsiimpl_;
  const ddk::GpioProtocolClient gpio_;
  uint32_t panel_type_;
  bool enabled_ = false;
};

}  // namespace mt8167s_display

#endif  // SRC_GRAPHICS_DISPLAY_DRIVERS_MT8167S_DISPLAY_LCD_H_
