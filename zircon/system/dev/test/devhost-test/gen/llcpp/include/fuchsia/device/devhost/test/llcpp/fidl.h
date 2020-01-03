// WARNING: This file is machine generated by fidlgen.

#pragma once

#include <lib/fidl/internal.h>
#include <lib/fidl/txn_header.h>
#include <lib/fidl/llcpp/array.h>
#include <lib/fidl/llcpp/coding.h>
#include <lib/fidl/llcpp/connect_service.h>
#include <lib/fidl/llcpp/service_handler_interface.h>
#include <lib/fidl/llcpp/string_view.h>
#include <lib/fidl/llcpp/sync_call.h>
#include <lib/fidl/llcpp/traits.h>
#include <lib/fidl/llcpp/transaction.h>
#include <lib/fidl/llcpp/vector_view.h>
#include <lib/fit/function.h>
#include <lib/zx/channel.h>
#include <zircon/fidl.h>

namespace llcpp {

namespace fuchsia {
namespace device {
namespace devhost {
namespace test {

struct TestDevice_AddChildDevice_Response;
struct TestDevice_AddChildDevice_Result;
class TestDevice;

extern "C" const fidl_type_t fuchsia_device_devhost_test_TestDevice_AddChildDevice_ResultTable;
extern "C" const fidl_type_t v1_fuchsia_device_devhost_test_TestDevice_AddChildDevice_ResultTable;

struct TestDevice_AddChildDevice_Result {
  TestDevice_AddChildDevice_Result() : ordinal_(Ordinal::Invalid), envelope_{} {}

  enum class Tag : fidl_xunion_tag_t {
    kResponse = 1,  // 0x1
    kErr = 2,  // 0x2
  };

  bool has_invalid_tag() const { return ordinal_ == Ordinal::Invalid; }

  bool is_response() const { return ordinal() == Ordinal::kResponse; }

  static TestDevice_AddChildDevice_Result WithResponse(::llcpp::fuchsia::device::devhost::test::TestDevice_AddChildDevice_Response* val) {
    TestDevice_AddChildDevice_Result result;
    result.set_response(val);
    return result;
  }

  void set_response(::llcpp::fuchsia::device::devhost::test::TestDevice_AddChildDevice_Response* elem) {
    ordinal_ = Ordinal::kResponse;
    envelope_.data = static_cast<void*>(elem);
  }

  ::llcpp::fuchsia::device::devhost::test::TestDevice_AddChildDevice_Response& mutable_response() {
    ZX_ASSERT(ordinal() == Ordinal::kResponse);
    return *static_cast<::llcpp::fuchsia::device::devhost::test::TestDevice_AddChildDevice_Response*>(envelope_.data);
  }
  const ::llcpp::fuchsia::device::devhost::test::TestDevice_AddChildDevice_Response& response() const {
    ZX_ASSERT(ordinal() == Ordinal::kResponse);
    return *static_cast<::llcpp::fuchsia::device::devhost::test::TestDevice_AddChildDevice_Response*>(envelope_.data);
  }

  bool is_err() const { return ordinal() == Ordinal::kErr; }

  static TestDevice_AddChildDevice_Result WithErr(int32_t* val) {
    TestDevice_AddChildDevice_Result result;
    result.set_err(val);
    return result;
  }

  void set_err(int32_t* elem) {
    ordinal_ = Ordinal::kErr;
    envelope_.data = static_cast<void*>(elem);
  }

  int32_t& mutable_err() {
    ZX_ASSERT(ordinal() == Ordinal::kErr);
    return *static_cast<int32_t*>(envelope_.data);
  }
  const int32_t& err() const {
    ZX_ASSERT(ordinal() == Ordinal::kErr);
    return *static_cast<int32_t*>(envelope_.data);
  }
  Tag which() const {
    ZX_ASSERT(!has_invalid_tag());
    return static_cast<Tag>(ordinal());
  }

  static constexpr const fidl_type_t* Type = &v1_fuchsia_device_devhost_test_TestDevice_AddChildDevice_ResultTable;
  static constexpr const fidl_type_t* AltType = &fuchsia_device_devhost_test_TestDevice_AddChildDevice_ResultTable;
  static constexpr uint32_t MaxNumHandles = 0;
  static constexpr uint32_t PrimarySize = 24;
  [[maybe_unused]]
  static constexpr uint32_t MaxOutOfLine = 8;
  static constexpr uint32_t AltPrimarySize = 24;
  [[maybe_unused]]
  static constexpr uint32_t AltMaxOutOfLine = 8;

 private:
  enum class Ordinal : fidl_xunion_tag_t {
    Invalid = 0,
    kResponse = 1,  // 0x1
    kErr = 2,  // 0x2
  };

  Ordinal ordinal() const {
    return ordinal_;
  }

  static void SizeAndOffsetAssertionHelper();
  Ordinal ordinal_;
  FIDL_ALIGNDECL
  fidl_envelope_t envelope_;
};

extern "C" const fidl_type_t fuchsia_device_devhost_test_TestDevice_AddChildDevice_ResponseTable;
extern "C" const fidl_type_t v1_fuchsia_device_devhost_test_TestDevice_AddChildDevice_ResponseTable;

struct TestDevice_AddChildDevice_Response {
  static constexpr const fidl_type_t* Type = &v1_fuchsia_device_devhost_test_TestDevice_AddChildDevice_ResponseTable;
  static constexpr const fidl_type_t* AltType = &fuchsia_device_devhost_test_TestDevice_AddChildDevice_ResponseTable;
  static constexpr uint32_t MaxNumHandles = 0;
  static constexpr uint32_t PrimarySize = 1;
  [[maybe_unused]]
  static constexpr uint32_t MaxOutOfLine = 0;
  static constexpr uint32_t AltPrimarySize = 1;
  [[maybe_unused]]
  static constexpr uint32_t AltMaxOutOfLine = 0;

  uint8_t __reserved = {};
};

extern "C" const fidl_type_t fuchsia_device_devhost_test_TestDeviceAddChildDeviceRequestTable;
extern "C" const fidl_type_t v1_fuchsia_device_devhost_test_TestDeviceAddChildDeviceRequestTable;
extern "C" const fidl_type_t fuchsia_device_devhost_test_TestDeviceAddChildDeviceResponseTable;
extern "C" const fidl_type_t v1_fuchsia_device_devhost_test_TestDeviceAddChildDeviceResponseTable;

class TestDevice final {
  TestDevice() = delete;
 public:

  struct AddChildDeviceResponse final {
    FIDL_ALIGNDECL
    fidl_message_header_t _hdr;
    ::llcpp::fuchsia::device::devhost::test::TestDevice_AddChildDevice_Result result;

    static constexpr const fidl_type_t* Type = &v1_fuchsia_device_devhost_test_TestDeviceAddChildDeviceResponseTable;
    static constexpr const fidl_type_t* AltType = &fuchsia_device_devhost_test_TestDeviceAddChildDeviceResponseTable;
    static constexpr uint32_t MaxNumHandles = 0;
    static constexpr uint32_t PrimarySize = 40;
    static constexpr uint32_t MaxOutOfLine = 8;
    static constexpr uint32_t AltPrimarySize = 24;
    static constexpr uint32_t AltMaxOutOfLine = 8;
    static constexpr bool HasFlexibleEnvelope = false;
    static constexpr bool ContainsUnion = true;
    static constexpr ::fidl::internal::TransactionalMessageKind MessageKind =
        ::fidl::internal::TransactionalMessageKind::kResponse;
  };
  using AddChildDeviceRequest = ::fidl::AnyZeroArgMessage;


  // Collection of return types of FIDL calls in this interface.
  class ResultOf final {
    ResultOf() = delete;
   private:
    template <typename ResponseType>
    class AddChildDevice_Impl final : private ::fidl::internal::OwnedSyncCallBase<ResponseType> {
      using Super = ::fidl::internal::OwnedSyncCallBase<ResponseType>;
     public:
      AddChildDevice_Impl(::zx::unowned_channel _client_end);
      ~AddChildDevice_Impl() = default;
      AddChildDevice_Impl(AddChildDevice_Impl&& other) = default;
      AddChildDevice_Impl& operator=(AddChildDevice_Impl&& other) = default;
      using Super::status;
      using Super::error;
      using Super::ok;
      using Super::Unwrap;
      using Super::value;
      using Super::operator->;
      using Super::operator*;
    };

   public:
    using AddChildDevice = AddChildDevice_Impl<AddChildDeviceResponse>;
  };

  // Collection of return types of FIDL calls in this interface,
  // when the caller-allocate flavor or in-place call is used.
  class UnownedResultOf final {
    UnownedResultOf() = delete;
   private:
    template <typename ResponseType>
    class AddChildDevice_Impl final : private ::fidl::internal::UnownedSyncCallBase<ResponseType> {
      using Super = ::fidl::internal::UnownedSyncCallBase<ResponseType>;
     public:
      AddChildDevice_Impl(::zx::unowned_channel _client_end, ::fidl::BytePart _response_buffer);
      ~AddChildDevice_Impl() = default;
      AddChildDevice_Impl(AddChildDevice_Impl&& other) = default;
      AddChildDevice_Impl& operator=(AddChildDevice_Impl&& other) = default;
      using Super::status;
      using Super::error;
      using Super::ok;
      using Super::Unwrap;
      using Super::value;
      using Super::operator->;
      using Super::operator*;
    };

   public:
    using AddChildDevice = AddChildDevice_Impl<AddChildDeviceResponse>;
  };

  class SyncClient final {
   public:
    explicit SyncClient(::zx::channel channel) : channel_(std::move(channel)) {}
    ~SyncClient() = default;
    SyncClient(SyncClient&&) = default;
    SyncClient& operator=(SyncClient&&) = default;

    const ::zx::channel& channel() const { return channel_; }

    ::zx::channel* mutable_channel() { return &channel_; }

    // Add child test device
    // Allocates 48 bytes of message buffer on the stack. No heap allocation necessary.
    ResultOf::AddChildDevice AddChildDevice();

    // Add child test device
    // Caller provides the backing storage for FIDL message via request and response buffers.
    UnownedResultOf::AddChildDevice AddChildDevice(::fidl::BytePart _response_buffer);

   private:
    ::zx::channel channel_;
  };

  // Methods to make a sync FIDL call directly on an unowned channel, avoiding setting up a client.
  class Call final {
    Call() = delete;
   public:

    // Add child test device
    // Allocates 48 bytes of message buffer on the stack. No heap allocation necessary.
    static ResultOf::AddChildDevice AddChildDevice(::zx::unowned_channel _client_end);

    // Add child test device
    // Caller provides the backing storage for FIDL message via request and response buffers.
    static UnownedResultOf::AddChildDevice AddChildDevice(::zx::unowned_channel _client_end, ::fidl::BytePart _response_buffer);

  };

  // Messages are encoded and decoded in-place when these methods are used.
  // Additionally, requests must be already laid-out according to the FIDL wire-format.
  class InPlace final {
    InPlace() = delete;
   public:

    // Add child test device
    static ::fidl::DecodeResult<AddChildDeviceResponse> AddChildDevice(::zx::unowned_channel _client_end, ::fidl::BytePart response_buffer);

  };

  // Pure-virtual interface to be implemented by a server.
  class Interface {
   public:
    Interface() = default;
    virtual ~Interface() = default;
    using _Outer = TestDevice;
    using _Base = ::fidl::CompleterBase;

    class AddChildDeviceCompleterBase : public _Base {
     public:
      void Reply(::llcpp::fuchsia::device::devhost::test::TestDevice_AddChildDevice_Result result);
      void ReplySuccess();
      void ReplyError(int32_t error);
      void Reply(::fidl::BytePart _buffer, ::llcpp::fuchsia::device::devhost::test::TestDevice_AddChildDevice_Result result);
      void ReplySuccess(::fidl::BytePart _buffer);
      void Reply(::fidl::DecodedMessage<AddChildDeviceResponse> params);

     protected:
      using ::fidl::CompleterBase::CompleterBase;
    };

    using AddChildDeviceCompleter = ::fidl::Completer<AddChildDeviceCompleterBase>;

    virtual void AddChildDevice(AddChildDeviceCompleter::Sync _completer) = 0;

  };

  // Attempts to dispatch the incoming message to a handler function in the server implementation.
  // If there is no matching handler, it returns false, leaving the message and transaction intact.
  // In all other cases, it consumes the message and returns true.
  // It is possible to chain multiple TryDispatch functions in this manner.
  static bool TryDispatch(Interface* impl, fidl_msg_t* msg, ::fidl::Transaction* txn);

  // Dispatches the incoming message to one of the handlers functions in the interface.
  // If there is no matching handler, it closes all the handles in |msg| and closes the channel with
  // a |ZX_ERR_NOT_SUPPORTED| epitaph, before returning false. The message should then be discarded.
  static bool Dispatch(Interface* impl, fidl_msg_t* msg, ::fidl::Transaction* txn);

  // Same as |Dispatch|, but takes a |void*| instead of |Interface*|. Only used with |fidl::Bind|
  // to reduce template expansion.
  // Do not call this method manually. Use |Dispatch| instead.
  static bool TypeErasedDispatch(void* impl, fidl_msg_t* msg, ::fidl::Transaction* txn) {
    return Dispatch(static_cast<Interface*>(impl), msg, txn);
  }


  // Helper functions to fill in the transaction header in a |DecodedMessage<TransactionalMessage>|.
  class SetTransactionHeaderFor final {
    SetTransactionHeaderFor() = delete;
   public:
    static void AddChildDeviceRequest(const ::fidl::DecodedMessage<TestDevice::AddChildDeviceRequest>& _msg);
    static void AddChildDeviceResponse(const ::fidl::DecodedMessage<TestDevice::AddChildDeviceResponse>& _msg);
  };
};

}  // namespace test
}  // namespace devhost
}  // namespace device
}  // namespace fuchsia
}  // namespace llcpp

namespace fidl {

template <>
struct IsFidlType<::llcpp::fuchsia::device::devhost::test::TestDevice_AddChildDevice_Response> : public std::true_type {};
static_assert(std::is_standard_layout_v<::llcpp::fuchsia::device::devhost::test::TestDevice_AddChildDevice_Response>);
static_assert(offsetof(::llcpp::fuchsia::device::devhost::test::TestDevice_AddChildDevice_Response, __reserved) == 0);
static_assert(sizeof(::llcpp::fuchsia::device::devhost::test::TestDevice_AddChildDevice_Response) == ::llcpp::fuchsia::device::devhost::test::TestDevice_AddChildDevice_Response::PrimarySize);

template <>
struct IsFidlType<::llcpp::fuchsia::device::devhost::test::TestDevice_AddChildDevice_Result> : public std::true_type {};
static_assert(std::is_standard_layout_v<::llcpp::fuchsia::device::devhost::test::TestDevice_AddChildDevice_Result>);

template <>
struct IsFidlType<::llcpp::fuchsia::device::devhost::test::TestDevice::AddChildDeviceResponse> : public std::true_type {};
template <>
struct IsFidlMessage<::llcpp::fuchsia::device::devhost::test::TestDevice::AddChildDeviceResponse> : public std::true_type {};
static_assert(sizeof(::llcpp::fuchsia::device::devhost::test::TestDevice::AddChildDeviceResponse)
    == ::llcpp::fuchsia::device::devhost::test::TestDevice::AddChildDeviceResponse::PrimarySize);
static_assert(offsetof(::llcpp::fuchsia::device::devhost::test::TestDevice::AddChildDeviceResponse, result) == 16);

}  // namespace fidl
