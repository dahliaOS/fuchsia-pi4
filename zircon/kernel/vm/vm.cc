// Copyright 2016 The Fuchsia Authors
// Copyright (c) 2014 Travis Geiselbrecht
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT
#include "vm/vm.h"

#include <align.h>
#include <assert.h>
#include <debug.h>
#include <inttypes.h>
#include <lib/console.h>
#include <lib/crypto/global_prng.h>
#include <lib/instrumentation/asan.h>
#include <lib/zircon-internal/macros.h>
#include <string.h>
#include <trace.h>
#include <zircon/errors.h>
#include <zircon/types.h>

#include <fbl/algorithm.h>
#include <kernel/thread.h>
#include <ktl/array.h>
#include <vm/bootalloc.h>
#include <vm/init.h>
#include <vm/physmap.h>
#include <vm/pmm.h>
#include <vm/vm_address_region.h>
#include <vm/vm_aspace.h>
#include <vm/vm_object_paged.h>

#include "vm_priv.h"

#define LOCAL_TRACE VM_GLOBAL_TRACE(0)

// boot time allocated page full of zeros
vm_page_t* zero_page;
paddr_t zero_page_paddr;

// set early in arch code to record the start address of the kernel
paddr_t kernel_base_phys;

// construct an array of kernel program segment descriptors for use here
// and elsewhere
namespace {
const ktl::array _kernel_regions = {
    kernel_region{
        .name = "kernel_code",
        .base = (vaddr_t)__code_start,
        .size = ROUNDUP((uintptr_t)__code_end - (uintptr_t)__code_start, PAGE_SIZE),
        .arch_mmu_flags = ARCH_MMU_FLAG_PERM_READ | ARCH_MMU_FLAG_PERM_EXECUTE | ARCH_MMU_FLAG_PERM_WRITE,
    },
    kernel_region{
        .name = "kernel_rodata",
        .base = (vaddr_t)__rodata_start,
        .size = ROUNDUP((uintptr_t)__rodata_end - (uintptr_t)__rodata_start, PAGE_SIZE),
        .arch_mmu_flags = ARCH_MMU_FLAG_PERM_READ | ARCH_MMU_FLAG_PERM_WRITE,
    },
    kernel_region{
        .name = "kernel_data",
        .base = (vaddr_t)__data_start,
        .size = ROUNDUP((uintptr_t)__data_end - (uintptr_t)__data_start, PAGE_SIZE),
        .arch_mmu_flags = ARCH_MMU_FLAG_PERM_READ | ARCH_MMU_FLAG_PERM_WRITE,
    },
    kernel_region{
        .name = "kernel_bss",
        .base = (vaddr_t)__bss_start,
        .size = ROUNDUP((uintptr_t)_end - (uintptr_t)__bss_start, PAGE_SIZE),
        .arch_mmu_flags = ARCH_MMU_FLAG_PERM_READ | ARCH_MMU_FLAG_PERM_WRITE,
    },
};
}  // namespace
const ktl::span<const kernel_region> kernel_regions{_kernel_regions};

namespace {

// mark a range of physical pages as WIRED
void MarkPagesInUsePhys(paddr_t pa, size_t len) {
  LTRACEF("pa %#" PRIxPTR ", len %#zx\n", pa, len);

  // make sure we are inclusive of all of the pages in the address range
  len = PAGE_ALIGN(len + (pa & (PAGE_SIZE - 1)));
  pa = ROUNDDOWN(pa, PAGE_SIZE);

  LTRACEF("aligned pa %#" PRIxPTR ", len %#zx\n", pa, len);

  list_node list = LIST_INITIAL_VALUE(list);

  zx_status_t status = pmm_alloc_range(pa, len / PAGE_SIZE, &list);
  ASSERT_MSG(status == ZX_OK, "failed to reserve memory range [%#" PRIxPTR ", %#" PRIxPTR "]\n", pa,
             pa + len - 1);

  // mark all of the pages we allocated as WIRED
  vm_page_t* p;
  list_for_every_entry (&list, p, vm_page_t, queue_node) { p->set_state(VM_PAGE_STATE_WIRED); }
}

}  // namespace

void vm_init_preheap() {
  LTRACE_ENTRY;

  // allow the vmm a shot at initializing some of its data structures
  VmAspace::KernelAspaceInitPreHeap();

  // mark the physical pages used by the boot time allocator
  if (boot_alloc_end != boot_alloc_start) {
    dprintf(INFO, "VM: marking boot alloc used range [%#" PRIxPTR ", %#" PRIxPTR ")\n",
            boot_alloc_start, boot_alloc_end);

    MarkPagesInUsePhys(boot_alloc_start, boot_alloc_end - boot_alloc_start);
  }

  zx_status_t status;

#if !DISABLE_KASLR  // Disable random memory padding for KASLR
  // Reserve up to 15 pages as a random padding in the kernel physical mapping
  unsigned char entropy;
  crypto::GlobalPRNG::GetInstance()->Draw(&entropy, sizeof(entropy));
  struct list_node list;
  list_initialize(&list);
  size_t page_count = entropy % 16;
  status = pmm_alloc_pages(page_count, 0, &list);
  DEBUG_ASSERT(status == ZX_OK);
  LTRACEF("physical mapping padding page count %#" PRIxPTR "\n", page_count);
#endif

  // grab a page and mark it as the zero page
  status = pmm_alloc_page(0, &zero_page, &zero_page_paddr);
  DEBUG_ASSERT(status == ZX_OK);

  void* ptr = paddr_to_physmap(zero_page_paddr);
  DEBUG_ASSERT(ptr);

  arch_zero_page(ptr);
}

void vm_init() {
  LTRACE_ENTRY;

  // Protect the regions of the physmap that are not backed by normal memory.
  //
  // See the comments for |phsymap_protect_non_arena_regions| for why we're doing this.
  //
#if defined(__aarch64__)
  physmap_protect_non_arena_regions();
#elif defined(__x86_64__)
  // TODO(fxbug.dev/48018): Call this on x64.  On x64, we access some non-arena parts of the physmap
  // (e.g. for smbios) so we can't change their protection.  Track down and remove these
  // dependencies so we can unify the arm64 and x64 paths.
#elif defined(__riscv)
#else
#error "unsupported architecture"
#endif

  VmAspace* aspace = VmAspace::kernel_aspace();

  fbl::RefPtr<VmAddressRegion> kernel_region;
  // | kernel_region_size | is the size in bytes of the region of memory occupied by the kernel
  // program's various segments (code, rodata, data, bss, etc.), inclusive of any gaps between
  // them.
  size_t kernel_region_size = get_kernel_size();
  // Create a VMAR that covers the address space occupied by the kernel program segments (code,
  // rodata, data, bss ,etc.). By creating this VMAR, we are effectively marking these addresses as
  // off limits to the VM. That way, the VM won't inadverantly use them for something else. This is
  // consistent with the initial mapping in start.S where the whole kernel region mapping was
  // written into the page table.
  //
  // Note: Even though there might be usable gaps in between the segments, we're covering the whole
  // regions. The thinking is that it's both simpler and safer to not use the address space that
  // exists between kernel program segments.
  zx_status_t status = aspace->RootVmar()->CreateSubVmar(
      kernel_regions[0].base - aspace->RootVmar()->base(), kernel_region_size, 0,
      VMAR_FLAG_CAN_MAP_SPECIFIC | VMAR_FLAG_SPECIFIC | VMAR_CAN_RWX_FLAGS, "kernel region vmar",
      &kernel_region);
  ASSERT(status == ZX_OK);

  for (const auto& region : kernel_regions) {
    ASSERT(IS_PAGE_ALIGNED(region.base));

    dprintf(INFO,
            "VM: reserving kernel region [%#" PRIxPTR ", %#" PRIxPTR ") flags %#x name '%s'\n",
            region.base, region.base + region.size, region.arch_mmu_flags, region.name);
    status =
        kernel_region->ReserveSpace(region.name, region.base, region.size, region.arch_mmu_flags);
    ASSERT(status == ZX_OK);

#if __has_feature(address_sanitizer)
    asan_remap_shadow(region.base, region.size);
#endif  // __has_feature(address_sanitizer)
  }

  // reserve the kernel aspace where the physmap is
  status = aspace->RootVmar()->ReserveSpace("physmap", PHYSMAP_BASE, PHYSMAP_SIZE,
                                            ARCH_MMU_FLAG_PERM_READ | ARCH_MMU_FLAG_PERM_WRITE);
  ASSERT(status == ZX_OK);

#if !DISABLE_KASLR  // Disable random memory padding for KASLR
  // Reserve random padding of up to 64GB after first mapping. It will make
  // the adjacent memory mappings (kstack_vmar, arena:handles and others) at
  // non-static virtual addresses.
  size_t entropy;
  crypto::GlobalPRNG::GetInstance()->Draw(&entropy, sizeof(entropy));

  size_t random_size = PAGE_ALIGN(entropy % (64ULL * GB));
  status = aspace->RootVmar()->ReserveSpace("random_padding", PHYSMAP_BASE + PHYSMAP_SIZE,
                                            random_size, 0);
  ASSERT(status == ZX_OK);
  LTRACEF("VM: aspace random padding size: %#" PRIxPTR "\n", random_size);
#endif
}

paddr_t vaddr_to_paddr(const void* ptr) {
  if (is_physmap_addr(ptr)) {
    return physmap_to_paddr(ptr);
  }

  auto aspace = VmAspace::vaddr_to_aspace(reinterpret_cast<uintptr_t>(ptr));
  if (!aspace) {
    return (paddr_t) nullptr;
  }

  paddr_t pa;
  zx_status_t rc = aspace->arch_aspace().Query((vaddr_t)ptr, &pa, nullptr);
  if (rc) {
    return (paddr_t) nullptr;
  }

  return pa;
}

static int cmd_vm(int argc, const cmd_args* argv, uint32_t flags) {
  if (argc < 2) {
  notenoughargs:
    printf("not enough arguments\n");
  usage:
    printf("usage:\n");
    printf("%s phys2virt <address>\n", argv[0].str);
    printf("%s virt2phys <address>\n", argv[0].str);
    printf("%s map <phys> <virt> <count> <flags>\n", argv[0].str);
    printf("%s unmap <virt> <count>\n", argv[0].str);
    return ZX_ERR_INTERNAL;
  }

  if (!strcmp(argv[1].str, "phys2virt")) {
    if (argc < 3) {
      goto notenoughargs;
    }

    if (!is_physmap_phys_addr(argv[2].u)) {
      printf("address isn't in physmap\n");
      return -1;
    }

    void* ptr = paddr_to_physmap((paddr_t)argv[2].u);
    printf("paddr_to_physmap returns %p\n", ptr);
  } else if (!strcmp(argv[1].str, "virt2phys")) {
    if (argc < 3) {
      goto notenoughargs;
    }

    VmAspace* aspace = VmAspace::vaddr_to_aspace(argv[2].u);
    if (!aspace) {
      printf("ERROR: outside of any address space\n");
      return -1;
    }

    paddr_t pa;
    uint flags;
    zx_status_t err = aspace->arch_aspace().Query(argv[2].u, &pa, &flags);
    printf("arch_mmu_query returns %d\n", err);
    if (err >= 0) {
      printf("\tpa %#" PRIxPTR ", flags %#x\n", pa, flags);
    }
  } else if (!strcmp(argv[1].str, "map")) {
    if (argc < 6) {
      goto notenoughargs;
    }

    VmAspace* aspace = VmAspace::vaddr_to_aspace(argv[2].u);
    if (!aspace) {
      printf("ERROR: outside of any address space\n");
      return -1;
    }

    size_t mapped;
    auto err = aspace->arch_aspace().MapContiguous(argv[3].u, argv[2].u, (uint)argv[4].u,
                                                   (uint)argv[5].u, &mapped);
    printf("arch_mmu_map returns %d, mapped %zu\n", err, mapped);
  } else if (!strcmp(argv[1].str, "unmap")) {
    if (argc < 4) {
      goto notenoughargs;
    }

    VmAspace* aspace = VmAspace::vaddr_to_aspace(argv[2].u);
    if (!aspace) {
      printf("ERROR: outside of any address space\n");
      return -1;
    }

    size_t unmapped;
    auto err = aspace->arch_aspace().Unmap(argv[2].u, (uint)argv[3].u, &unmapped);
    printf("arch_mmu_unmap returns %d, unmapped %zu\n", err, unmapped);
  } else {
    printf("unknown command\n");
    goto usage;
  }

  return ZX_OK;
}

STATIC_COMMAND_START
STATIC_COMMAND("vm", "vm commands", &cmd_vm)
STATIC_COMMAND_END(vm)
