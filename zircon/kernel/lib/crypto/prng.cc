// Copyright 2016 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#include <assert.h>
#include <lib/crypto/prng.h>
#include <lib/fit/defer.h>
#include <pow2.h>
#include <string.h>
#include <zircon/compiler.h>
#include <zircon/errors.h>
#include <zircon/types.h>

#include <explicit-memory/bytes.h>
#include <kernel/mutex.h>
#include <ktl/atomic.h>

// See note in //zircon/third_party/ulib/boringssl/BUILD.gn
#define BORINGSSL_NO_CXX
#include <openssl/chacha.h>
#include <openssl/sha.h>

namespace crypto {
namespace {

const uint128_t kNonceOverflow = ((uint128_t)1ULL) << 96;

}  // namespace

PRNG::PRNG(const void* data, size_t size) : PRNG(data, size, NonThreadSafeTag()) {
  static_assert(sizeof(key_) == SHA256_DIGEST_LENGTH, "Bad key length");
  BecomeThreadSafe();
}

PRNG::PRNG(const void* data, size_t size, NonThreadSafeTag tag) : nonce_(0), accumulated_(0) {
  memset(key_, 0, sizeof(key_));
  memset(&ready_, 0, sizeof(ready_));
  AddEntropy(data, size);
}

PRNG::~PRNG() {
  mandatory_memset(key_, 0, sizeof(key_));
  nonce_ = 0;
}

void PRNG::AddEntropy(const void* data, size_t size) {
  DEBUG_ASSERT(data || size == 0);
  ASSERT(size <= kMaxEntropy);
  // Concurrent calls to |AddEntropy| must run sequentially.
  Guard<Mutex> guard(&mutex_);
  // Save the key on the stack, but guarantee we clean them up
  uint8_t key[sizeof(key_)];
  auto cleanup = fit::defer([&] { mandatory_memset(key, 0, sizeof(key)); });
  // We mix all of the entropy with the previous key to make the PRNG state
  // depend on both the entropy added and the sequence in which it was added.
  SHA256_CTX ctx;
  SHA256_Init(&ctx);
  SHA256_Update(&ctx, data, size);
  {
    Guard<SpinLock, IrqSave> guard(&spinlock_);
    memcpy(key, key_, sizeof(key));
  }
  SHA256_Update(&ctx, key, sizeof(key));
  SHA256_Final(key, &ctx);
  {
    Guard<SpinLock, IrqSave> guard(&spinlock_);
    memcpy(key_, key, sizeof(key_));
  }
  // Increment how much entropy has been added, and signal if we have enough.
  size_t total_entropy = accumulated_.fetch_add(size) + size;
  if (is_thread_safe() && total_entropy >= kMinEntropy) {
    ready_->Signal();
  }
}

// AddEntropy() with NULL input effectively reseeds with hash of current key.
void PRNG::SelfReseed() { AddEntropy(NULL, 0); }

void PRNG::Draw(void* out, size_t size) {
  DEBUG_ASSERT(out || size == 0);
  ASSERT(size <= kMaxDrawLen);
  // Wait if other threads should add entropy.
  if (is_thread_safe() && accumulated_.load() < kMinEntropy) {
    ready_->Wait();
  }
  // Save these on the stack, but guarantee we clean them up
  uint8_t key[sizeof(key_)];
  uint128_t nonce;
  auto cleanup = fit::defer([&] {
    mandatory_memset(key, 0, sizeof(key));
    nonce = 0;
  });
  {
    Guard<SpinLock, IrqSave> guard(&spinlock_);
    nonce = ++nonce_;
    memcpy(key, key_, sizeof(key));
  }
  ASSERT(nonce < kNonceOverflow);
  static_assert(__BYTE_ORDER__ == __ORDER_LITTLE_ENDIAN__, "must be LE");
  uint8_t* nonce_u8 = reinterpret_cast<uint8_t*>(&nonce);
  uint8_t* buf = reinterpret_cast<uint8_t*>(out);

  // We randomize |buf| by encrypting it with a key that is never exposed to
  // the caller, and a 96-bit nonce that changes on each call.  We don't zero
  // |buf| because the encrypted output meets the criteria of the PRNG
  // regardless of its original contents.  We reset the counter to 0 on each
  // request; it can't overflow because of the limit on the overall size.
  CRYPTO_chacha_20(buf, buf, size, key, nonce_u8, 0);
}

uint64_t PRNG::RandInt(uint64_t exclusive_upper_bound) {
  ASSERT(exclusive_upper_bound != 0);

  const uint log2 = log2_ulong_ceil(exclusive_upper_bound);
  const size_t mask =
      (log2 != sizeof(uint64_t) * CHAR_BIT) ? (uint64_t(1) << log2) - 1 : UINT64_MAX;
  DEBUG_ASSERT(exclusive_upper_bound - 1 <= mask);

  // This loop should terminate very fast, since the probability that the
  // drawn value is >= exclusive_upper_bound is less than 0.5.  This is the
  // classic discard out-of-range values approach.
  while (true) {
    uint64_t v;
    Draw(reinterpret_cast<uint8_t*>(&v), sizeof(uint64_t) / sizeof(uint8_t));
    v &= mask;
    if (v < exclusive_upper_bound) {
      return v;
    }
  }
}

// It is safe to call this function from PRNG's constructor provided
// |ready_| and |accumulated_| initialized.
void PRNG::BecomeThreadSafe() {
  ASSERT(!is_thread_safe());
  ready_.Initialize(accumulated_.load() >= kMinEntropy);
  is_thread_safe_ = true;
}

bool PRNG::is_thread_safe() const {
  // Safe to read |is_thread_safe_|; it is read-only in a threaded context.
  return is_thread_safe_;
}

}  // namespace crypto
