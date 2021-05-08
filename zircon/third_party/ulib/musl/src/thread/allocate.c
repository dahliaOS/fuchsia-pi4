#include <stdatomic.h>
#include <stddef.h>
#include <string.h>
#include <zircon/process.h>
#include <zircon/syscalls.h>

#include "libc.h"
#include "threads_impl.h"
#include "zircon_impl.h"

// See dynlink.c for the full explanation.  The compiler generates calls to
// these implicitly.  They are PLT calls into the ASan runtime, which is fine
// in and of itself at this point (unlike in dynlink.c).  But they might also
// use ShadowCallStack, which is not set up yet.  So make sure references here
// only use the libc-internal symbols, which don't have any setup requirements.
#if __has_feature(address_sanitizer)
__asm__(".weakref __asan_memcpy,__libc_memcpy");
__asm__(".weakref __asan_memset,__libc_memset");
#endif

#ifdef __aarch64__
// Clang's <arm_acle.h> has __yield() but GCC doesn't (nor the intrinsic).
__NO_SAFESTACK static inline void relax(void) { __asm__("yield"); }
#elif defined(__x86_64__)
// GCC's <x86intrin.h> has __pause() but not Clang though it has the intrinsic.
__NO_SAFESTACK static inline void relax(void) { __builtin_ia32_pause(); }
#else
__NO_SAFESTACK static inline void relax(void) {}
#endif

static struct pthread* all_threads;
static atomic_flag all_threads_lock = ATOMIC_FLAG_INIT;

__NO_SAFESTACK struct pthread** __thread_list_acquire(void) {
  while (atomic_flag_test_and_set_explicit(&all_threads_lock, memory_order_acquire)) {
    relax();
  }
  return &all_threads;
}

__NO_SAFESTACK void __thread_list_release(void) {
  atomic_flag_clear_explicit(&all_threads_lock, memory_order_release);
}

// A detached thread has to remove itself from the list.
// Joinable threads get removed only in pthread_join.
__NO_SAFESTACK void __thread_list_erase(void* arg) {
  struct pthread* t = arg;
  __thread_list_acquire();
  *t->prevp = t->next;
  if (t->next != NULL) {
    t->next->prevp = t->prevp;
  }
  __thread_list_release();
}

static pthread_rwlock_t allocation_lock = PTHREAD_RWLOCK_INITIALIZER;

// Many threads could be reading the TLS state.
static void thread_allocation_acquire(void) { pthread_rwlock_rdlock(&allocation_lock); }

// dlopen calls this under another lock. Only one dlopen call can be
// modifying state at a time.
void __thread_allocation_inhibit(void) { pthread_rwlock_wrlock(&allocation_lock); }

void __thread_allocation_release(void) { pthread_rwlock_unlock(&allocation_lock); }

__NO_SAFESTACK static inline size_t round_up_to_page(size_t sz) {
  return (sz + PAGE_SIZE - 1) & -PAGE_SIZE;
}

__NO_SAFESTACK static ptrdiff_t offset_for_module(const struct tls_module* module) {
#ifdef TLS_ABOVE_TP
  return module->offset;
#else
  return -module->offset;
#endif
}

__NO_SAFESTACK static thrd_t copy_tls(unsigned char* mem, size_t alloc) {
  thrd_t td;
  struct tls_module* p;
  size_t i;
  void** dtv;

#ifdef TLS_ABOVE_TP
  // *-----------------------------------------------------------------------*
  // | pthread | tcb | X | tls_1 | ... | tlsN | ... | tls_cnt | dtv[1] | ... |
  // *-----------------------------------------------------------------------*
  // ^         ^         ^             ^            ^
  // td        tp      dtv[1]       dtv[n+1]       dtv
  //
  // Note: The TCB is actually the last member of pthread.
  // See: "Addenda to, and Errata in, the ABI for the ARM Architecture"

  dtv = (void**)(mem + libc.tls_size) - (libc.tls_cnt + 1);
  // We need to make sure that the thread pointer is maximally aligned so
  // that tp + dtv[N] is aligned to align_N no matter what N is. So we need
  // 'mem' to be such that if mem == td then td->head is maximially aligned.
  // To do this we need take &td->head (e.g. mem + offset of head) and align
  // it then subtract out the offset of ->head to ensure that &td->head is
  // aligned.
  uintptr_t tp = (uintptr_t)mem + PTHREAD_TP_OFFSET;
  tp = (tp + libc.tls_align - 1) & -libc.tls_align;
  td = (thrd_t)(tp - PTHREAD_TP_OFFSET);
  // Now mem should be the new thread pointer.
  mem = (unsigned char*)tp;
#else
  // *-----------------------------------------------------------------------*
  // | tls_cnt | dtv[1] | ... | tls_n | ... | tls_1 | tcb | pthread | unused |
  // *-----------------------------------------------------------------------*
  // ^                        ^             ^       ^
  // dtv                   dtv[n+1]       dtv[1]  tp/td
  //
  // Note: The TCB is actually the first member of pthread.
  dtv = (void**)mem;

  mem += alloc - sizeof(struct pthread);
  mem -= (uintptr_t)mem & (libc.tls_align - 1);
  td = (thrd_t)mem;
#endif

  for (i = 1, p = libc.tls_head; p; i++, p = p->next) {
    dtv[i] = mem + offset_for_module(p) + DTP_OFFSET;
    memcpy(mem + offset_for_module(p), p->image, p->len);
  }

  dtv[0] = (void*)libc.tls_cnt;
  td->head.dtv = dtv;
  return td;
}

__NO_SAFESTACK static bool map_block(zx_handle_t parent_vmar, zx_handle_t vmo, size_t vmo_offset,
                                     size_t size, size_t before, size_t after,
                                     struct iovec* mapping, struct iovec* region) {
  region->iov_len = before + size + after;
  zx_handle_t vmar;
  uintptr_t addr;
  zx_status_t status = _zx_vmar_allocate(
      parent_vmar, ZX_VM_CAN_MAP_READ | ZX_VM_CAN_MAP_WRITE | ZX_VM_CAN_MAP_SPECIFIC, 0,
      region->iov_len, &vmar, &addr);
  if (status != ZX_OK)
    return true;
  region->iov_base = (void*)addr;
  status = _zx_vmar_map(vmar, ZX_VM_PERM_READ | ZX_VM_PERM_WRITE | ZX_VM_SPECIFIC, before, vmo,
                        vmo_offset, size, &addr);
  if (status != ZX_OK)
    _zx_vmar_destroy(vmar);
  _zx_handle_close(vmar);
  mapping->iov_base = (void*)addr;
  mapping->iov_len = size;
  return status != ZX_OK;
}

// This allocates all the per-thread memory for a new thread about to
// be created, or for the initial thread at startup.  It's called
// either at startup or under thread_allocation_acquire.  Hence,
// it's serialized with any dynamic linker changes to the TLS
// bookkeeping.
//
// This conceptually allocates five things, but concretely allocates
// four separate blocks.
// 1. The safe stack (where the thread's SP will point).
// 2. The unsafe stack (where __builtin___get_unsafe_stack_ptr() will point).
// 3. The shadow call stack (where the thread's SCSP will point).
//    (This only exists #if HAVE_SHADOW_CALL_STACK.)
// 4. The thread descriptor (struct pthread).  The thread pointer points
//    into this (where into it depends on the machine ABI).
// 5. The static TLS area.  The ELF TLS ABI for the Initial Exec model
//    mandates a fixed distance from the thread pointer to the TLS area
//    across all threads.  So effectively this must always be allocated
//    as part of the same block with the thread descriptor.
// This function also copies in the TLS initializer data.
// It initializes the basic thread descriptor fields.
// Everything else is zero-initialized.
//
// The region for the TCB and TLS area has a precise required size that's
// computed here.  The sizes of the stacks and the guard regions around them
// are speculative parameters to be tuned.  Note that there are only two tuning
// knobs provided due to API legacy: the "stack size" and the "guard size".
//
// Nowadays with both safe-stack and shadow-call-stack available in the ABI
// there are three different stacks to choose sizes for.  Different kinds of
// program behavior consume each of the different stacks at different rates, so
// it's hard to predict generically: buffers and other address-taken stack
// variables grow the unsafe stack; pure call depth (e.g. deep recursion) grows
// the shadow call stack; certain kinds of large functions, and aggregate call
// depth of those, grow the safe stack.
//
// The legacy presumption is that all consumption is on a single stack (the
// machine stack, aka the "safe" stack under safe-stack).  Thus the single
// tuned size provided by the legacy API is meant to represent total
// consumption across all types of stack use but we don't know how best to
// allot that among the three stacks so that the actual overall consumption
// pattern that works in the traditional single-stack ABI with a given total
// consumption limit still works in with the new stack ABIs.
//
// To support whatever consumption patterns may arise, we give each of the
// three stacks the full size requested via the legacy API for a unitary stack.
// This seems very wasteful: 3x the stack allocation!  But in theory it should
// only waste 3x *address space*, not 3x *memory*.  The worst-case total
// "wasted" space in each of the three should be one page minus one word,
// i.e. around three pages total (plus some amortized page table overhead
// proportional to the address space use).  Since all stack pages are actually
// lazily allocated on demand, the excess unused pages of each stack that's
// larger than it needs to be will never be allocated.  The only alternative
// that works in the general case is to come up with new tuning APIs that can
// express the different kinds of stack consumption required to tune the three
// sizes separately (or proportionally to each other or whatever).

// In the function below, the compiler may generate calls to memcpy
// intrinsics for copying structs. With ASan enabled, calls to these memcpy
// intrinsics are converted to calls to __asan_memcpy. Calls to the ASan runtime
// in these cases may not be safe because of ABI requirements like
// ShadowCallStack that aren't ready yet. So redirect this symbol to libc's own
// memcpy implementation, which is always a leaf function that doesn't require
// the ShadowCallStack ABI.
#if __has_feature(address_sanitizer)
__asm__(".weakref __asan_memcpy, __unsanitized_memcpy");
#endif

__NO_SAFESTACK thrd_t __allocate_thread(size_t requested_guard_size, size_t requested_stack_size,
                                        const char* thread_name, char vmo_name[ZX_MAX_NAME_LEN]) {
  // In the initial thread, we're allocating the stacks and TCB for the running
  // thread itself.  So we can't make calls that rely on safe-stack or
  // shadow-call-stack setup.  Rather than annotating everything in the call
  // path here, we just avoid the problematic calls.  Locking is not required
  // since this is the sole thread.
  const bool initial_thread = vmo_name == NULL;

  if (!initial_thread) {
    thread_allocation_acquire();
  }

  const size_t guard_size = requested_guard_size == 0 ? 0 : round_up_to_page(requested_guard_size);
  const size_t stack_size = round_up_to_page(requested_stack_size);

  const size_t tls_size = libc.tls_size;
  const size_t tcb_size = round_up_to_page(tls_size);

  const size_t vmo_size = tcb_size + stack_size * (2 + HAVE_SHADOW_CALL_STACK);
  zx_handle_t vmo;
  zx_status_t status = _zx_vmo_create(vmo_size, 0, &vmo);
  if (status != ZX_OK) {
    if (!initial_thread) {
      __thread_allocation_release();
    }
    return NULL;
  }
  struct iovec tcb, tcb_region;
  if (map_block(_zx_vmar_root_self(), vmo, 0, tcb_size, PAGE_SIZE, PAGE_SIZE, &tcb, &tcb_region)) {
    if (!initial_thread) {
      __thread_allocation_release();
    }
    _zx_handle_close(vmo);
    return NULL;
  }

  thrd_t td = copy_tls(tcb.iov_base, tcb.iov_len);

  // At this point all our access to global TLS state is done, so we
  // can allow dlopen again.
  if (!initial_thread) {
    __thread_allocation_release();
  }

  // For the initial thread, it's too early to call snprintf because
  // it's not __NO_SAFESTACK.
  if (!initial_thread) {
    // For other threads, try to give the VMO a name that includes
    // the thrd_t value (and the TLS size if that fits too), but
    // don't use a truncated value since that would be confusing to
    // interpret.
    if (snprintf(vmo_name, ZX_MAX_NAME_LEN, "%s:%p/TLS=%#zx", thread_name, td, tls_size) <
            ZX_MAX_NAME_LEN ||
        snprintf(vmo_name, ZX_MAX_NAME_LEN, "%s:%p", thread_name, td) < ZX_MAX_NAME_LEN)
      thread_name = vmo_name;
  }
  _zx_object_set_property(vmo, ZX_PROP_NAME, thread_name, strlen(thread_name));

  if (map_block(_zx_vmar_root_self(), vmo, tcb_size, stack_size, guard_size, 0, &td->safe_stack,
                &td->safe_stack_region)) {
    _zx_vmar_unmap(_zx_vmar_root_self(), (uintptr_t)tcb_region.iov_base, tcb_region.iov_len);
    _zx_handle_close(vmo);
    return NULL;
  }

  if (map_block(_zx_vmar_root_self(), vmo, tcb_size + stack_size, stack_size, guard_size, 0,
                &td->unsafe_stack, &td->unsafe_stack_region)) {
    _zx_vmar_unmap(_zx_vmar_root_self(), (uintptr_t)td->safe_stack_region.iov_base,
                   td->safe_stack_region.iov_len);
    _zx_vmar_unmap(_zx_vmar_root_self(), (uintptr_t)tcb_region.iov_base, tcb_region.iov_len);
    _zx_handle_close(vmo);
    return NULL;
  }

#if HAVE_SHADOW_CALL_STACK
  if (map_block(_zx_vmar_root_self(), vmo, tcb_size + stack_size * 2,
                // Shadow call stack grows up, so a guard after is probably
                // enough.  But be extra careful with guards on both sides.
                stack_size, guard_size, guard_size,
                //
                &td->shadow_call_stack, &td->shadow_call_stack_region)) {
    _zx_vmar_unmap(_zx_vmar_root_self(), (uintptr_t)td->unsafe_stack_region.iov_base,
                   td->unsafe_stack_region.iov_len);
    _zx_vmar_unmap(_zx_vmar_root_self(), (uintptr_t)td->safe_stack_region.iov_base,
                   td->safe_stack_region.iov_len);
    _zx_vmar_unmap(_zx_vmar_root_self(), (uintptr_t)tcb_region.iov_base, tcb_region.iov_len);
    _zx_handle_close(vmo);
    return NULL;
  }
#endif

  _zx_handle_close(vmo);
  td->tcb_region = tcb_region;
  td->locale = &libc.global_locale;
  td->head.tp = (uintptr_t)pthread_to_tp(td);
  td->abi.stack_guard = __stack_chk_guard;
  td->abi.unsafe_sp = (uintptr_t)td->unsafe_stack.iov_base + td->unsafe_stack.iov_len;

  struct pthread** prevp = __thread_list_acquire();
  td->prevp = prevp;
  td->next = *prevp;
  if (td->next != NULL) {
    td->next->prevp = &td->next;
  }
  *prevp = td;
  __thread_list_release();

  return td;
}
