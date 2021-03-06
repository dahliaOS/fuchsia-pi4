// Copyright 2021 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include "src/ui/bin/root_presenter/tests/fakes/fake_view.h"

#include <lib/ui/scenic/cpp/view_ref_pair.h>
#include <lib/ui/scenic/cpp/view_token_pair.h>

namespace root_presenter::testing {

FakeView::FakeView(sys::ComponentContext* component_context, fuchsia::ui::scenic::ScenicPtr scenic)
    : scenic_(std::move(scenic)), session_(scenic_.get()) {
  // Set up session listener event handler.
  session_.set_event_handler([this](std::vector<fuchsia::ui::scenic::Event> events) {
    for (const auto& event : events) {
      events_.emplace_back();
      fidl::Clone(event, &events_.back());
    }
  });

  auto [view_token, view_holder_token] = scenic::ViewTokenPair::New();
  view_holder_token_ = std::move(view_holder_token);
  auto [control_ref, view_ref] = scenic::ViewRefPair::New();
  fake_view_.emplace(&session_, std::move(view_token), std::move(control_ref), std::move(view_ref),
                     "Fake View");

  // Apply changes.
  session_.Present(/* presentation_time = */ 0,
                   /* presentation_callback = */ [](fuchsia::images::PresentationInfo info) {});
}

fuchsia::ui::views::ViewHolderToken FakeView::view_holder_token() const {
  fuchsia::ui::views::ViewHolderToken copy;
  fidl::Clone(view_holder_token_.value(), &copy);
  return copy;
}

std::optional<uint32_t> FakeView::view_id() const {
  if (!fake_view_) {
    return std::nullopt;
  }

  return std::optional<uint32_t>(fake_view_->id());
}

}  // namespace root_presenter::testing
