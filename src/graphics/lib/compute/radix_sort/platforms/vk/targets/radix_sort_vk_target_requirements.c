// Copyright 2021 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include "radix_sort_vk_target_requirements.h"

#include <stdio.h>

#include "common/macros.h"
#include "radix_sort/platforms/vk/radix_sort_vk.h"
#include "radix_sort_vk_target.h"
#include "target_archive/target_archive.h"

//
//
//

struct radix_sort_vk_target
{
  struct target_archive_header ar_header;
};

//
// RADIX_SORT TARGET REQUIREMENTS: VULKAN
//

bool
radix_sort_vk_target_get_requirements(struct radix_sort_vk_target const *        target,
                                      struct radix_sort_vk_target_requirements * requirements)
{
  //
  // Must not be NULL
  //
  if ((target == NULL) || (requirements == NULL))
    {
      return false;
    }

#ifndef RADIX_SORT_VK_DISABLE_VERIFY
  //
  // Verify target archive is valid archive
  //
  if (target->ar_header.magic != TARGET_ARCHIVE_MAGIC)
    {
#ifndef NDEBUG
      fprintf(stderr, "Error: Invalid target -- missing magic.");
#endif
      return NULL;
    }
#endif

  //
  // Get the Radix Sort target header.
  //
  struct target_archive_header const * const ar_header  = &target->ar_header;
  struct target_archive_entry const * const  ar_entries = ar_header->entries;
  uint32_t const * const                     ar_data    = ar_entries[ar_header->count - 1].data;

  union
  {
    struct radix_sort_vk_target_header const * header;
    uint32_t const *                           data;
  } const rs_target = { .data = ar_data };

#ifndef RADIX_SORT_VK_DISABLE_VERIFY
  //
  // Verify target is compatible with the library.
  //
  if (rs_target.header->config.magic != RS_CONFIG_MAGIC)
    {
#ifndef NDEBUG
      fprintf(stderr, "Error: Target is not compatible with library.");
#endif
      return NULL;
    }

    //
    // TODO(allanmac): Verify `ar_header->count` but note that not all target
    // archives will have a static count.
    //
#endif

  //
  //
  //
  bool is_success = true;

  //
  // EXTENSIONS
  //
  {
    //
    // Compute number of required extensions
    //
    uint32_t ext_count = 0;

    for (uint32_t ii = 0; ii < ARRAY_LENGTH_MACRO(rs_target.header->extensions.bitmap); ii++)
      {
        ext_count += __builtin_popcount(rs_target.header->extensions.bitmap[ii]);
      }

    if (requirements->ext_names == NULL)
      {
        requirements->ext_name_count = ext_count;

        if (ext_count > 0)
          {
            is_success = false;
          }
      }
    else
      {
        if (requirements->ext_name_count < ext_count)
          {
            is_success = false;
          }
        else
          {
            //
            // FIXME(allanmac): This can be accelerated by exploiting
            // the extension bitmap.
            //
            uint32_t ii = 0;

#define RADIX_SORT_VK_TARGET_EXTENSION_STRING(ext_) "VK_" STRINGIFY_MACRO(ext_)

#undef RADIX_SORT_VK_TARGET_EXTENSION
#define RADIX_SORT_VK_TARGET_EXTENSION(ext_)                                                       \
  if (rs_target.header->extensions.named.ext_)                                                     \
    {                                                                                              \
      requirements->ext_names[ii++] = RADIX_SORT_VK_TARGET_EXTENSION_STRING(ext_);                 \
    }

            RADIX_SORT_VK_TARGET_EXTENSIONS()
          }
      }
  }

  //
  // Enable physical device features
  //
  if ((requirements->pdf == NULL) || (requirements->pdf11 == NULL) || (requirements->pdf12 == NULL))
    {
      is_success = false;
    }
  else
    {
#undef RADIX_SORT_VK_TARGET_FEATURE_VK10
#define RADIX_SORT_VK_TARGET_FEATURE_VK10(feature_) 1 +

#undef RADIX_SORT_VK_TARGET_FEATURE_VK11
#define RADIX_SORT_VK_TARGET_FEATURE_VK11(feature_) 1 +

#undef RADIX_SORT_VK_TARGET_FEATURE_VK12
#define RADIX_SORT_VK_TARGET_FEATURE_VK12(feature_) 1 +

      //
      // Don't create the variable if it's not used
      //
#if (RADIX_SORT_VK_TARGET_FEATURES_VK10() 0)
      VkPhysicalDeviceFeatures * const pdf = requirements->pdf;
#endif
#if (RADIX_SORT_VK_TARGET_FEATURES_VK11() 0)
      VkPhysicalDeviceVulkan11Features * const pdf11 = requirements->pdf11;
#endif
#if (RADIX_SORT_VK_TARGET_FEATURES_VK12() 0)
      VkPhysicalDeviceVulkan12Features * const pdf12 = requirements->pdf12;
#endif

      //
      // Let's always have this on during debug
      //
#ifndef NDEBUG
      pdf->robustBufferAccess = true;
#endif

      //
      // VULKAN 1.0
      //
#undef RADIX_SORT_VK_TARGET_FEATURE_VK10
#define RADIX_SORT_VK_TARGET_FEATURE_VK10(feature_)                                                \
  if (rs_target.header->features.named.feature_)                                                   \
    pdf->feature_ = true;

      RADIX_SORT_VK_TARGET_FEATURES_VK10()

      //
      // VULKAN 1.1
      //
#undef RADIX_SORT_VK_TARGET_FEATURE_VK11
#define RADIX_SORT_VK_TARGET_FEATURE_VK11(feature_)                                                \
  if (rs_target.header->features.named.feature_)                                                   \
    pdf11->feature_ = true;

      RADIX_SORT_VK_TARGET_FEATURES_VK11()

      //
      // VULKAN 1.2
      //
#undef RADIX_SORT_VK_TARGET_FEATURE_VK12
#define RADIX_SORT_VK_TARGET_FEATURE_VK12(feature_)                                                \
  if (rs_target.header->features.named.feature_)                                                   \
    pdf12->feature_ = true;

      RADIX_SORT_VK_TARGET_FEATURES_VK12()
    }

  //
  //
  //
  return is_success;
}

//
//
//
