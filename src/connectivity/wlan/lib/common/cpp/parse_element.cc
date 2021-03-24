// Copyright 2018 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include <fuchsia/wlan/ieee80211/cpp/fidl.h>

#include <wlan/common/buffer_reader.h>
#include <wlan/common/from_bytes.h>
#include <wlan/common/parse_element.h>

namespace wlan_ieee80211 = ::fuchsia::wlan::ieee80211;

namespace wlan {
namespace common {

namespace {
template <typename T>
const T* ParseFixedSized(fbl::Span<const uint8_t> raw_body) {
  if (raw_body.size() != sizeof(T)) {
    return nullptr;
  }
  return reinterpret_cast<const T*>(raw_body.data());
}
}  // namespace

std::optional<fbl::Span<const uint8_t>> ParseSsid(fbl::Span<const uint8_t> raw_body) {
  if (raw_body.size() > wlan_ieee80211::MAX_SSID_LEN) {
    return {};
  }
  return {raw_body};
}

std::optional<fbl::Span<const SupportedRate>> ParseSupportedRates(
    fbl::Span<const uint8_t> raw_body) {
  if (raw_body.empty() || raw_body.size() > kMaxSupportedRatesLen) {
    return {};
  }
  auto rates = reinterpret_cast<const SupportedRate*>(raw_body.data());
  static_assert(sizeof(SupportedRate) == sizeof(uint8_t));
  return {{rates, raw_body.size()}};
}

const DsssParamSet* ParseDsssParamSet(fbl::Span<const uint8_t> raw_body) {
  return ParseFixedSized<DsssParamSet>(raw_body);
}

const CfParamSet* ParseCfParamSet(fbl::Span<const uint8_t> raw_body) {
  return ParseFixedSized<CfParamSet>(raw_body);
}

std::optional<ParsedTim> ParseTim(fbl::Span<const uint8_t> raw_body) {
  auto header = FromBytes<TimHeader>(raw_body);
  if (header == nullptr) {
    return {};
  }
  auto bitmap = raw_body.subspan(sizeof(TimHeader));
  if (bitmap.empty()) {
    return {};
  }
  return {{*header, bitmap}};
}

std::optional<ParsedCountry> ParseCountry(fbl::Span<const uint8_t> raw_body) {
  auto country = FromBytes<Country>(raw_body);
  if (country == nullptr) {
    return {};
  }
  auto remaining = raw_body.subspan(sizeof(Country));
  size_t num_triplets = remaining.size() / sizeof(SubbandTriplet);
  return {{*country, {reinterpret_cast<const SubbandTriplet*>(remaining.data()), num_triplets}}};
}

std::optional<fbl::Span<const SupportedRate>> ParseExtendedSupportedRates(
    fbl::Span<const uint8_t> raw_body) {
  if (raw_body.empty()) {
    return {};
  }
  auto rates = reinterpret_cast<const SupportedRate*>(raw_body.data());
  static_assert(sizeof(SupportedRate) == sizeof(uint8_t));
  return {{rates, raw_body.size()}};
}

const MeshConfiguration* ParseMeshConfiguration(fbl::Span<const uint8_t> raw_body) {
  return ParseFixedSized<MeshConfiguration>(raw_body);
}

std::optional<fbl::Span<const uint8_t>> ParseMeshId(fbl::Span<const uint8_t> raw_body) {
  if (raw_body.size() > kMaxMeshIdLen) {
    return {};
  }
  return {raw_body};
}

const QosInfo* ParseQosCapability(fbl::Span<const uint8_t> raw_body) {
  return ParseFixedSized<QosInfo>(raw_body);
}

const common::MacAddr* ParseGcrGroupAddress(fbl::Span<const uint8_t> raw_body) {
  return ParseFixedSized<common::MacAddr>(raw_body);
}

const HtCapabilities* ParseHtCapabilities(fbl::Span<const uint8_t> raw_body) {
  return ParseFixedSized<HtCapabilities>(raw_body);
}

const HtOperation* ParseHtOperation(fbl::Span<const uint8_t> raw_body) {
  return ParseFixedSized<HtOperation>(raw_body);
}

const VhtCapabilities* ParseVhtCapabilities(fbl::Span<const uint8_t> raw_body) {
  return ParseFixedSized<VhtCapabilities>(raw_body);
}

const VhtOperation* ParseVhtOperation(fbl::Span<const uint8_t> raw_body) {
  return ParseFixedSized<VhtOperation>(raw_body);
}

std::optional<ParsedMpmOpen> ParseMpmOpen(fbl::Span<const uint8_t> raw_body) {
  auto r = BufferReader{raw_body};
  auto header = r.ReadValue<MpmHeader>();
  if (!header) {
    return {};
  }

  auto pmk = r.Read<MpmPmk>();
  if (r.RemainingBytes() > 0) {
    return {};
  }

  return {{*header, pmk}};
}

std::optional<ParsedMpmConfirm> ParseMpmConfirm(fbl::Span<const uint8_t> raw_body) {
  auto r = BufferReader{raw_body};
  auto header = r.ReadValue<MpmHeader>();
  if (!header) {
    return {};
  }

  auto peer_link_id = r.ReadValue<uint16_t>();
  if (!peer_link_id) {
    return {};
  }

  auto pmk = r.Read<MpmPmk>();
  if (r.RemainingBytes() > 0) {
    return {};
  }

  return {{*header, *peer_link_id, pmk}};
}

std::optional<ParsedMpmClose> ParseMpmClose(fbl::Span<const uint8_t> raw_body) {
  auto r = BufferReader{raw_body};
  auto header = r.ReadValue<MpmHeader>();
  if (!header) {
    return {};
  }

  auto peer_link_id = std::optional<uint16_t>{};
  if (r.RemainingBytes() % sizeof(MpmPmk) == 2 * sizeof(uint16_t)) {
    peer_link_id = r.ReadValue<uint16_t>();
  }

  auto reason_code = r.ReadValue<uint16_t>();
  if (!reason_code) {
    return {};
  }

  auto pmk = r.Read<MpmPmk>();
  if (r.RemainingBytes() > 0) {
    return {};
  }

  return {{*header, peer_link_id, *reason_code, pmk}};
}

std::optional<ParsedPreq> ParsePreq(fbl::Span<const uint8_t> raw_body) {
  ParsedPreq ret{};
  auto r = BufferReader{raw_body};
  ret.header = r.Read<PreqHeader>();
  if (ret.header == nullptr) {
    return {};
  }

  if (ret.header->flags.addr_ext()) {
    ret.originator_external_addr = r.Read<common::MacAddr>();
    if (ret.originator_external_addr == nullptr) {
      return {};
    }
  }

  ret.middle = r.Read<PreqMiddle>();
  if (ret.middle == nullptr) {
    return {};
  }

  ret.per_target = r.ReadArray<PreqPerTarget>(ret.middle->target_count);
  if (ret.per_target.size() != ret.middle->target_count) {
    return {};
  }

  if (r.RemainingBytes() > 0) {
    return {};
  }

  return {ret};
}

std::optional<ParsedPrep> ParsePrep(fbl::Span<const uint8_t> raw_body) {
  ParsedPrep ret{};
  auto r = BufferReader{raw_body};
  ret.header = r.Read<PrepHeader>();
  if (ret.header == nullptr) {
    return {};
  }

  if (ret.header->flags.addr_ext()) {
    ret.target_external_addr = r.Read<common::MacAddr>();
    if (ret.target_external_addr == nullptr) {
      return {};
    }
  }

  ret.tail = r.Read<PrepTail>();
  if (ret.tail == nullptr) {
    return {};
  }

  if (r.RemainingBytes() > 0) {
    return {};
  }

  return {ret};
}

std::optional<ParsedPerr> ParsePerr(fbl::Span<const uint8_t> raw_body) {
  ParsedPerr ret{};
  auto r = BufferReader{raw_body};

  ret.header = r.Read<PerrHeader>();
  if (ret.header == nullptr) {
    return {};
  }

  ret.destinations = r.ReadRemaining();
  return {ret};
}

}  // namespace common
}  // namespace wlan
