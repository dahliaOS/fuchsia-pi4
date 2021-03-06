// Copyright 2020 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include "src/storage/lib/paver/chromebook-x64.h"

#include <set>

#include <chromeos-disk-setup/chromeos-disk-setup.h>
#include <gpt/cros.h>

#include "src/lib/uuid/uuid.h"
#include "src/storage/lib/paver/abr-client-vboot.h"
#include "src/storage/lib/paver/abr-client.h"
#include "src/storage/lib/paver/pave-logging.h"
#include "src/storage/lib/paver/utils.h"
#include "src/storage/lib/paver/validation.h"

namespace paver {

namespace {

using uuid::Uuid;

namespace block = fuchsia_hardware_block;

constexpr size_t kKibibyte = 1024;
constexpr size_t kMebibyte = kKibibyte * 1024;
constexpr size_t kGibibyte = kMebibyte * 1024;

// Chromebook uses the legacy partition scheme.
constexpr PartitionScheme kPartitionScheme = PartitionScheme::kLegacy;

zx::status<Uuid> CrosPartitionType(Partition type) {
  switch (type) {
    case Partition::kZirconA:
    case Partition::kZirconB:
    case Partition::kZirconR:
      return zx::ok(Uuid(GUID_CROS_KERNEL_VALUE));
    default:
      return GptPartitionType(type);
  }
}

}  // namespace

zx::status<std::unique_ptr<DevicePartitioner>> CrosDevicePartitioner::Initialize(
    fbl::unique_fd devfs_root, fidl::UnownedClientEnd<fuchsia_io::Directory> svc_root, Arch arch,
    const fbl::unique_fd& block_device) {
  if (arch != Arch::kX64) {
    return zx::error(ZX_ERR_NOT_FOUND);
  }

  zx::status<> status = IsBootloader(devfs_root, "coreboot");
  if (status.is_error()) {
    return status.take_error();
  }

  auto status_or_gpt =
      GptDevicePartitioner::InitializeGpt(std::move(devfs_root), svc_root, block_device);
  if (status_or_gpt.is_error()) {
    return status_or_gpt.take_error();
  }
  std::unique_ptr<GptDevicePartitioner>& gpt_partitioner = status_or_gpt->gpt;

  GptDevice* gpt = gpt_partitioner->GetGpt();
  block::wire::BlockInfo info = gpt_partitioner->GetBlockInfo();

  if (!is_ready_to_pave(gpt, reinterpret_cast<fuchsia_hardware_block_BlockInfo*>(&info),
                        SZ_ZX_PART)) {
    auto pauser = BlockWatcherPauser::Create(gpt_partitioner->svc_root());
    if (pauser.is_error()) {
      ERROR("Failed to pause the block watcher");
      return pauser.take_error();
    }

    auto status = zx::make_status(config_cros_for_fuchsia(
        gpt, reinterpret_cast<fuchsia_hardware_block_BlockInfo*>(&info), SZ_ZX_PART));
    if (status.is_error()) {
      ERROR("Failed to configure CrOS for Fuchsia.\n");
      return status.take_error();
    }
    if (auto status = zx::make_status(gpt->Sync()); status.is_error()) {
      ERROR("Failed to sync CrOS for Fuchsia.\n");
      return status.take_error();
    }
    __UNUSED auto unused =
        RebindGptDriver(gpt_partitioner->svc_root(), gpt_partitioner->Channel()).status_value();
  }

  // Determine if the firmware supports A/B in Zircon.
  auto active_slot = abr::QueryBootConfig(gpt_partitioner->devfs_root(), svc_root);
  bool supports_abr = active_slot.is_ok();

  auto partitioner =
      WrapUnique(new CrosDevicePartitioner(std::move(gpt_partitioner), supports_abr));
  if (status_or_gpt->initialize_partition_tables) {
    if (auto status = partitioner->InitPartitionTables(); status.is_error()) {
      return status.take_error();
    }
  }

  LOG("Successfully initialized CrOS Device Partitioner\n");
  return zx::ok(std::move(partitioner));
}

bool CrosDevicePartitioner::SupportsPartition(const PartitionSpec& spec) const {
  const PartitionSpec supported_specs[] = {PartitionSpec(paver::Partition::kZirconA),
                                           PartitionSpec(paver::Partition::kZirconB),
                                           PartitionSpec(paver::Partition::kZirconR),
                                           PartitionSpec(paver::Partition::kFuchsiaVolumeManager)};

  for (const auto& supported : supported_specs) {
    if (SpecMatches(spec, supported)) {
      return true;
    }
  }

  return false;
}

zx::status<std::unique_ptr<PartitionClient>> CrosDevicePartitioner::AddPartition(
    const PartitionSpec& spec) const {
  if (!SupportsPartition(spec)) {
    ERROR("Unsupported partition %s\n", spec.ToString().c_str());
    return zx::error(ZX_ERR_NOT_SUPPORTED);
  }

  // NOTE: If you update the minimum sizes of partitions, please update the
  // CrosDevicePartitionerTests.InitPartitionTables test.
  size_t minimum_size_bytes = 0;
  switch (spec.partition) {
    case Partition::kZirconA:
      minimum_size_bytes = 64 * kMebibyte;
      break;
    case Partition::kZirconB:
      minimum_size_bytes = 64 * kMebibyte;
      break;
    case Partition::kZirconR:
      // NOTE(abdulla): is_ready_to_pave() is called with SZ_ZX_PART, which requires all kernel
      // partitions to be the same size.
      minimum_size_bytes = 64 * kMebibyte;
      break;
    case Partition::kFuchsiaVolumeManager:
      minimum_size_bytes = 16 * kGibibyte;
      break;
    default:
      ERROR("Cros partitioner cannot add unknown partition type\n");
      return zx::error(ZX_ERR_NOT_SUPPORTED);
  }

  const char* name = PartitionName(spec.partition, kPartitionScheme);
  auto type = CrosPartitionType(spec.partition);
  if (type.is_error()) {
    return type.take_error();
  }
  return gpt_->AddPartition(name, type.value(), minimum_size_bytes, /*optional_reserve_bytes*/ 0);
}

zx::status<std::unique_ptr<PartitionClient>> CrosDevicePartitioner::FindPartition(
    const PartitionSpec& spec) const {
  if (!SupportsPartition(spec)) {
    ERROR("Unsupported partition %s\n", spec.ToString().c_str());
    return zx::error(ZX_ERR_NOT_SUPPORTED);
  }

  switch (spec.partition) {
    case Partition::kZirconA:
    case Partition::kZirconB:
    case Partition::kZirconR: {
      const auto filter = [&spec](const gpt_partition_t& part) {
        const char* name = PartitionName(spec.partition, kPartitionScheme);
        auto partition_type = CrosPartitionType(spec.partition);
        return partition_type.is_ok() && FilterByTypeAndName(part, partition_type.value(), name);
      };
      auto status = gpt_->FindPartition(filter);
      if (status.is_error()) {
        return status.take_error();
      }
      return zx::ok(std::move(status->partition));
    }
    case Partition::kFuchsiaVolumeManager: {
      auto status = gpt_->FindPartition(IsFvmPartition);
      if (status.is_error()) {
        return status.take_error();
      }
      return zx::ok(std::move(status->partition));
    }
    default:
      ERROR("Cros partitioner cannot find unknown partition type\n");
      return zx::error(ZX_ERR_NOT_SUPPORTED);
  }
}

zx::status<> CrosDevicePartitioner::FinalizePartition(const PartitionSpec& spec) const {
  if (!SupportsPartition(spec)) {
    ERROR("Unsupported partition %s\n", spec.ToString().c_str());
    return zx::error(ZX_ERR_NOT_SUPPORTED);
  }

  // Special partition finalization is only necessary for Zircon partitions, and only when we don't
  // support A/B.
  if (spec.partition != Partition::kZirconA || supports_abr_) {
    return zx::ok();
  }

  // Find the Zircon A kernel partition.
  const char* name = PartitionName(Partition::kZirconA, kPartitionScheme);
  const auto filter = [name](const gpt_partition_t& part) {
    return FilterByTypeAndName(part, GUID_CROS_KERNEL_VALUE, name);
  };
  auto status = gpt_->FindPartition(filter);
  if (status.is_error()) {
    ERROR("Cannot find %s partition\n", name);
    return status.take_error();
  }
  gpt_partition_t* zircon_a_partition = status->gpt_partition;

  // Find the highest priority kernel partition.
  uint8_t top_priority = 0;
  for (uint32_t i = 0; i < gpt::kPartitionCount; ++i) {
    zx::status<const gpt_partition_t*> partition_or = gpt_->GetGpt()->GetPartition(i);
    if (partition_or.is_error()) {
      continue;
    }
    const gpt_partition_t* partition = partition_or.value();
    const uint8_t priority = gpt_cros_attr_get_priority(partition->flags);
    // Ignore anything not of type CROS KERNEL.
    if (Uuid(partition->type) != Uuid(GUID_CROS_KERNEL_VALUE)) {
      continue;
    }

    // Ignore ourself.
    if (partition == zircon_a_partition) {
      continue;
    }

    if (priority > top_priority) {
      top_priority = priority;
    }
  }

  // Priority for Zircon A set to higher priority than all other kernels.
  if (top_priority == UINT8_MAX) {
    ERROR("Cannot set CrOS partition priority higher than other kernels\n");
    return zx::error(ZX_ERR_OUT_OF_RANGE);
  }
  int ret = gpt_cros_attr_set_priority(&zircon_a_partition->flags,
                                       static_cast<uint8_t>(top_priority + 1));
  if (ret != 0) {
    ERROR("Cannot set CrOS partition priority for ZIRCON-A\n");
    return zx::error(ZX_ERR_OUT_OF_RANGE);
  }

  // Successful set to 'true' to encourage the bootloader to
  // use this partition.
  gpt_cros_attr_set_successful(&zircon_a_partition->flags, true);
  // Maximize the number of attempts to boot this partition before
  // we fall back to a different kernel.
  ret = gpt_cros_attr_set_tries(&zircon_a_partition->flags, 15);
  if (ret != 0) {
    ERROR("Cannot set CrOS partition 'tries' for ZIRCON-A\n");
    return zx::error(ZX_ERR_OUT_OF_RANGE);
  }
  if (auto status = zx::make_status(gpt_->GetGpt()->Sync()); status.is_error()) {
    ERROR("Failed to sync CrOS partition 'tries' for ZIRCON-A\n");
    return status.take_error();
  }

  return zx::ok();
}

zx::status<> CrosDevicePartitioner::WipeFvm() const { return gpt_->WipeFvm(); }

zx::status<> CrosDevicePartitioner::InitPartitionTables() const {
  // Wipe partitions.
  // CrosDevicePartitioner operates on partition names.
  const std::set<std::string_view> partitions_to_wipe{
      GUID_ZIRCON_A_NAME,
      GUID_ZIRCON_B_NAME,
      GUID_ZIRCON_R_NAME,
      GUID_FVM_NAME,
      // These additional partition names are based on the previous naming scheme.
      "ZIRCON-A",
      "ZIRCON-B",
      "ZIRCON-R",
      "fvm",
  };
  auto status = gpt_->WipePartitions([&partitions_to_wipe](const gpt_partition_t& part) {
    char cstring_name[GPT_NAME_LEN] = {};
    utf16_to_cstring(cstring_name, part.name, GPT_NAME_LEN);
    return partitions_to_wipe.find(cstring_name) != partitions_to_wipe.end();
  });
  if (status.is_error()) {
    ERROR("Failed to wipe partitions: %s\n", status.status_string());
    return status.take_error();
  }

  // Add partitions with default content type.
  const std::array<PartitionSpec, 4> partitions_to_add = {
      PartitionSpec(Partition::kZirconA),
      PartitionSpec(Partition::kZirconB),
      PartitionSpec(Partition::kZirconR),
      PartitionSpec(Partition::kFuchsiaVolumeManager),
  };
  for (auto spec : partitions_to_add) {
    auto status = AddPartition(spec);
    if (status.is_error()) {
      ERROR("Failed to create partition \"%s\": %s\n", spec.ToString().c_str(),
            status.status_string());
      return status.take_error();
    }
  }

  LOG("Successfully initialized GPT\n");
  return zx::ok();
}

zx::status<> CrosDevicePartitioner::WipePartitionTables() const {
  return gpt_->WipePartitionTables();
}

zx::status<> CrosDevicePartitioner::ValidatePayload(const PartitionSpec& spec,
                                                    fbl::Span<const uint8_t> data) const {
  if (!SupportsPartition(spec)) {
    ERROR("Unsupported partition %s\n", spec.ToString().c_str());
    return zx::error(ZX_ERR_NOT_SUPPORTED);
  }

  if (IsZirconPartitionSpec(spec)) {
    if (!IsValidChromeOSKernel(data)) {
      return zx::error(ZX_ERR_BAD_STATE);
    }
  }

  return zx::ok();
}

zx::status<std::unique_ptr<DevicePartitioner>> ChromebookX64PartitionerFactory::New(
    fbl::unique_fd devfs_root, fidl::UnownedClientEnd<fuchsia_io::Directory> svc_root, Arch arch,
    std::shared_ptr<Context> context, const fbl::unique_fd& block_device) {
  return CrosDevicePartitioner::Initialize(std::move(devfs_root), svc_root, arch, block_device);
}

zx::status<std::unique_ptr<abr::Client>> ChromebookX64AbrClientFactory::New(
    fbl::unique_fd devfs_root, fidl::UnownedClientEnd<fuchsia_io::Directory> svc_root,
    std::shared_ptr<paver::Context> context) {
  fbl::unique_fd none;
  auto partitioner = CrosDevicePartitioner::Initialize(std::move(devfs_root), std::move(svc_root),
                                                       paver::Arch::kX64, none);

  if (partitioner.is_error()) {
    return partitioner.take_error();
  }

  return abr::VbootClient::Create(std::unique_ptr<CrosDevicePartitioner>(
      static_cast<CrosDevicePartitioner*>(partitioner.value().release())));
}

}  // namespace paver
