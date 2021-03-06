// Copyright 2021 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include <lib/fidl/llcpp/message.h>
#include <zircon/assert.h>
#include <zircon/syscalls.h>

#include <array>
#include <memory>
#include <string>

#include <gtest/gtest.h>

#include "src/lib/fidl/llcpp/tests/types_test_utils.h"

TEST(IncomingMessage, ConstructNonOkMessage) {
  constexpr auto kError = "test error";
  fidl::IncomingMessage message(fidl::Result::TransportError(ZX_ERR_ACCESS_DENIED, kError));
  EXPECT_FALSE(message.ok());
  EXPECT_EQ(ZX_ERR_ACCESS_DENIED, message.status());
}

TEST(IncomingMessage, ConstructNonOkMessageRequiresNonOkStatus) {
#if ZX_DEBUG_ASSERT_IMPLEMENTED
  ASSERT_DEATH({ fidl::IncomingMessage message(fidl::Result::DecodeError(ZX_OK)); }, "!= ZX_OK");
#else
  GTEST_SKIP() << "Debug assertions are disabled";
#endif
}

// This fixture checks that handles have been closed at the end of the test case.
class IncomingMessageWithHandlesTest : public ::testing::Test {
 protected:
  void SetUp() override {
    fidl_init_txn_header(reinterpret_cast<fidl_message_header_t*>(bytes_), /* txid */ 1,
                         /* ordinal */ 1);

    ASSERT_EQ(ZX_OK, zx_event_create(0, &event1_));
    checker_.AddEvent(event1_);
    ASSERT_EQ(ZX_OK, zx_event_create(0, &event2_));
    checker_.AddEvent(event2_);

    handles_[0] = zx_handle_info_t{
        .handle = event1_,
        .type = ZX_OBJ_TYPE_EVENT,
        .rights = ZX_RIGHTS_BASIC,
        .unused = 0,
    };
    handles_[1] = zx_handle_info_t{
        .handle = event2_,
        .type = ZX_OBJ_TYPE_EVENT,
        .rights = ZX_RIGHTS_BASIC,
        .unused = 0,
    };
  }

  void TearDown() override { checker_.CheckEvents(); }

  llcpp_types_test_utils::HandleChecker checker_;
  uint8_t bytes_[sizeof(fidl_message_header_t)] = {};
  zx_handle_t event1_;
  zx_handle_t event2_;
  zx_handle_info_t handles_[2];
};

TEST_F(IncomingMessageWithHandlesTest, AdoptHandlesFromC) {
  fidl_incoming_msg_t c_msg = {
      .bytes = bytes_,
      .handles = handles_,
      .num_bytes = static_cast<uint32_t>(std::size(bytes_)),
      .num_handles = static_cast<uint32_t>(std::size(handles_)),
  };
  auto incoming = fidl::IncomingMessage::FromEncodedCMessage(&c_msg);
  EXPECT_EQ(ZX_OK, incoming.status());
}

TEST_F(IncomingMessageWithHandlesTest, AdoptHandlesWithRegularConstructor) {
  auto incoming = fidl::IncomingMessage(bytes_, static_cast<uint32_t>(std::size(bytes_)), handles_,
                                        static_cast<uint32_t>(std::size(handles_)));
  EXPECT_EQ(ZX_OK, incoming.status());
}

TEST_F(IncomingMessageWithHandlesTest, ReleaseHandles) {
  fidl_incoming_msg c_msg = {};

  {
    auto incoming = fidl::IncomingMessage(bytes_, static_cast<uint32_t>(std::size(bytes_)),
                                          handles_, static_cast<uint32_t>(std::size(handles_)));
    ASSERT_EQ(ZX_OK, incoming.status());
    c_msg = std::move(incoming).ReleaseToEncodedCMessage();
    // At this point, |incoming| will not close the handles.
  }

  for (zx_handle_info_t event : handles_) {
    zx_info_handle_count_t info = {};
    zx_status_t status = zx_object_get_info(event.handle, ZX_INFO_HANDLE_COUNT, &info, sizeof(info),
                                            nullptr, nullptr);
    ASSERT_EQ(ZX_OK, status);
    EXPECT_GT(info.handle_count, 1U);
  }

  // Adopt the handles again to close them.
  auto incoming = fidl::IncomingMessage::FromEncodedCMessage(&c_msg);
}

TEST_F(IncomingMessageWithHandlesTest, MoveConstructorHandleOwnership) {
  auto incoming = fidl::IncomingMessage(bytes_, static_cast<uint32_t>(std::size(bytes_)), handles_,
                                        static_cast<uint32_t>(std::size(handles_)));
  fidl::IncomingMessage another{std::move(incoming)};
  EXPECT_EQ(incoming.handle_actual(), 0u);
  EXPECT_GT(another.handle_actual(), 0u);
  EXPECT_EQ(ZX_OK, another.status());
}

TEST(IncomingMessage, ValidateTransactionalMessageHeader) {
  uint8_t bytes[sizeof(fidl_message_header_t)] = {};
  auto* hdr = reinterpret_cast<fidl_message_header_t*>(bytes);
  fidl_init_txn_header(hdr, /* txid */ 1, /* ordinal */ 1);
  // Unsupported wire-format magic number.
  hdr->magic_number = 42;

  {
    auto incoming =
        fidl::IncomingMessage(bytes, static_cast<uint32_t>(std::size(bytes)), nullptr, 0);
    EXPECT_EQ(ZX_ERR_PROTOCOL_NOT_SUPPORTED, incoming.status());
    EXPECT_FALSE(incoming.ok());
  }

  {
    auto incoming = fidl::IncomingMessage(bytes, static_cast<uint32_t>(std::size(bytes)), nullptr,
                                          0, fidl::IncomingMessage::kSkipMessageHeaderValidation);
    EXPECT_EQ(ZX_OK, incoming.status());
    EXPECT_TRUE(incoming.ok());
  }
}

class IncomingMessageChannelReadEtcTest : public ::testing::Test {
 protected:
  void SetUp() override {
    byte_buffer_ = std::make_unique<std::array<uint8_t, ZX_CHANNEL_MAX_MSG_BYTES>>();
    handle_buffer_ = std::make_unique<std::array<zx_handle_info_t, ZX_CHANNEL_MAX_MSG_HANDLES>>();
  }

  fidl::BufferSpan byte_buffer_view() {
    return fidl::BufferSpan(byte_buffer_->data(), ZX_CHANNEL_MAX_MSG_HANDLES);
  }

  cpp20::span<zx_handle_info_t> handle_buffer_view() { return cpp20::span(*handle_buffer_); }

 private:
  std::unique_ptr<std::array<uint8_t, ZX_CHANNEL_MAX_MSG_BYTES>> byte_buffer_;
  std::unique_ptr<std::array<zx_handle_info_t, ZX_CHANNEL_MAX_MSG_HANDLES>> handle_buffer_;
};

TEST_F(IncomingMessageChannelReadEtcTest, ReadFromChannel) {
  zx::channel source, sink;
  ASSERT_EQ(ZX_OK, zx::channel::create(0, &source, &sink));

  uint8_t bytes[sizeof(fidl_message_header_t)] = {};
  auto* hdr = reinterpret_cast<fidl_message_header_t*>(bytes);
  fidl_init_txn_header(hdr, /* txid */ 1, /* ordinal */ 1);
  sink.write(0, bytes, std::size(bytes), nullptr, 0);

  auto incoming = fidl::ChannelReadEtc(source.get(), 0, byte_buffer_view(), handle_buffer_view());
  EXPECT_EQ(ZX_OK, incoming.status());
  EXPECT_EQ(incoming.byte_actual(), sizeof(fidl_message_header_t));
  EXPECT_EQ(0, memcmp(incoming.bytes(), bytes, incoming.byte_actual()));
  EXPECT_EQ(0u, incoming.handle_actual());

  auto incoming2 = fidl::ChannelReadEtc(source.get(), 0, byte_buffer_view(), handle_buffer_view());
  EXPECT_EQ(ZX_ERR_SHOULD_WAIT, incoming2.status());
  EXPECT_EQ(fidl::Reason::kTransportError, incoming2.reason());
  EXPECT_EQ(std::string(fidl::internal::kErrorTransport), std::string(incoming2.error_message()));
}

TEST_F(IncomingMessageChannelReadEtcTest, ReadFromClosedChannel) {
  zx::channel source, sink;
  ASSERT_EQ(ZX_OK, zx::channel::create(0, &source, &sink));

  sink.reset();
  auto incoming = fidl::ChannelReadEtc(source.get(), 0, byte_buffer_view(), handle_buffer_view());
  EXPECT_EQ(ZX_ERR_PEER_CLOSED, incoming.status());
  EXPECT_EQ(fidl::Reason::kPeerClosed, incoming.reason());
}

TEST_F(IncomingMessageChannelReadEtcTest, ReadFromChannelInvalidMessage) {
  zx::channel source, sink;
  ASSERT_EQ(ZX_OK, zx::channel::create(0, &source, &sink));

  uint8_t bytes[sizeof(fidl_message_header_t)] = {};
  auto* hdr = reinterpret_cast<fidl_message_header_t*>(bytes);
  // An epitaph must have zero txid, so the following is invalid.
  fidl_init_txn_header(hdr, /* txid */ 42, /* ordinal */ kFidlOrdinalEpitaph);
  sink.write(0, bytes, std::size(bytes), nullptr, 0);

  auto incoming = fidl::ChannelReadEtc(source.get(), 0, byte_buffer_view(), handle_buffer_view());
  EXPECT_EQ(ZX_ERR_INVALID_ARGS, incoming.status());
  EXPECT_EQ(fidl::Reason::kUnexpectedMessage, incoming.reason());
  EXPECT_EQ(std::string(fidl::internal::kErrorInvalidHeader),
            std::string(incoming.error_message()));
}
