// Copyright 2018 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include "src/storage/blobfs/blob_cache.h"

#include <zircon/compiler.h>

#include <iterator>
#include <memory>

#include <gtest/gtest.h>

#include "src/storage/blobfs/cache_node.h"
#include "src/storage/blobfs/test/unit/utils.h"

namespace blobfs {
namespace {

// A mock Node, comparable to Blob.
//
// "ShouldCache" mimics the internal Vnode state machine.
// "UsingMemory" mimics the storage of pages and mappings, which may be evicted
// from memory when references are closed.
class TestNode : public CacheNode, fbl::Recyclable<TestNode> {
 public:
  explicit TestNode(const Digest& digest, BlobCache* cache)
      : CacheNode(nullptr, digest), cache_(cache) {}

  void fbl_recycle() final { CacheNode::fbl_recycle(); }

  BlobCache& GetCache() final { return *cache_; }

  bool ShouldCache() const final { return should_cache_; }

  void ActivateLowMemory() final { using_memory_ = false; }

#if defined(ENABLE_BLOBFS_NEW_PAGER)
  // fs::PagedVnode implementation.
  void VmoRead(uint64_t offset, uint64_t length) override {
    ASSERT_TRUE(false);  // Should not get called in these tests.
  }
#endif

  bool UsingMemory() { return using_memory_; }

  void SetCache(bool should_cache) { should_cache_ = should_cache; }

  void SetHighMemory() { using_memory_ = true; }

  fs::VnodeProtocolSet GetProtocols() const final { return fs::VnodeProtocol::kFile; }

  zx_status_t GetNodeInfoForProtocol(fs::VnodeProtocol protocol, fs::Rights rights,
                                     fs::VnodeRepresentation* representation) {
    *representation = fs::VnodeRepresentation::File();
    return ZX_OK;
  }

 private:
  BlobCache* cache_;
  bool should_cache_ = true;
  bool using_memory_ = false;
};

Digest GenerateDigest(size_t seed) {
  Digest digest;
  digest.Init();
  digest.Update(&seed, sizeof(seed));
  digest.Final();
  return digest;
}

void CheckNothingOpenHelper(BlobCache* cache) {
  ASSERT_TRUE(cache);
  cache->ForAllOpenNodes([](fbl::RefPtr<CacheNode>) -> zx_status_t { ZX_ASSERT(false); });
}

TEST(BlobCacheTest, Null) {
  BlobCache cache;

  CheckNothingOpenHelper(&cache);
  cache.Reset();
  CheckNothingOpenHelper(&cache);

  Digest digest = GenerateDigest(0);
  fbl::RefPtr<CacheNode> missing_node;
  ASSERT_EQ(ZX_ERR_NOT_FOUND, cache.Lookup(digest, nullptr));
  ASSERT_EQ(ZX_ERR_NOT_FOUND, cache.Lookup(digest, &missing_node));
  auto node = fbl::MakeRefCounted<TestNode>(digest, &cache);
  ASSERT_EQ(ZX_ERR_NOT_FOUND, cache.Evict(node));
  node->SetCache(false);
}

TEST(BlobCacheTest, AddLookupEvict) {
  // Add a node to the cache.
  BlobCache cache;
  Digest digest = GenerateDigest(0);
  auto node = fbl::MakeRefCounted<TestNode>(digest, &cache);
  ASSERT_EQ(cache.Add(node), ZX_OK);
  ASSERT_EQ(ZX_ERR_ALREADY_EXISTS, cache.Add(node));

  // Observe that we can access the node inside the cache.
  fbl::RefPtr<CacheNode> found_node;
  ASSERT_EQ(cache.Lookup(digest, nullptr), ZX_OK);
  ASSERT_EQ(cache.Lookup(digest, &found_node), ZX_OK);
  ASSERT_EQ(found_node.get(), node.get());

  // Observe that evicting the node removes it from the cache.
  ASSERT_EQ(cache.Evict(node), ZX_OK);
  ASSERT_EQ(ZX_ERR_NOT_FOUND, cache.Lookup(digest, nullptr));
}

// ShouldCache = false, Evicted = false.
//
// This results in the node being deleted from the cache.
TEST(BlobCacheTest, StopCaching) {
  BlobCache cache;
  Digest digest = GenerateDigest(0);
  // The node is also deleted if we stop caching it, instead of just evicting.
  {
    auto node = fbl::MakeRefCounted<TestNode>(digest, &cache);
    ASSERT_EQ(cache.Add(node), ZX_OK);
    ASSERT_EQ(cache.Lookup(digest, nullptr), ZX_OK);
    node->SetCache(false);
  }
  ASSERT_EQ(ZX_ERR_NOT_FOUND, cache.Lookup(digest, nullptr));
}

// ShouldCache = false, Evicted = True.
//
// This results in the node being deleted from the cache.
TEST(BlobCacheTest, EvictNoCache) {
  BlobCache cache;
  Digest digest = GenerateDigest(0);
  // The node is also deleted if we stop caching it, instead of just evicting.
  {
    auto node = fbl::MakeRefCounted<TestNode>(digest, &cache);
    ASSERT_EQ(cache.Add(node), ZX_OK);
    ASSERT_EQ(cache.Lookup(digest, nullptr), ZX_OK);
    ASSERT_EQ(cache.Evict(node), ZX_OK);
  }
  ASSERT_EQ(ZX_ERR_NOT_FOUND, cache.Lookup(digest, nullptr));
}

// ShouldCache = true, Evicted = true.
//
// This results in the node being deleted from the cache.
TEST(BlobCacheTest, EvictWhileCaching) {
  BlobCache cache;
  Digest digest = GenerateDigest(0);
  // The node is automatically deleted if it wants to be cached, but has been
  // evicted.
  {
    auto node = fbl::MakeRefCounted<TestNode>(digest, &cache);
    ASSERT_EQ(cache.Add(node), ZX_OK);
    ASSERT_EQ(cache.Lookup(digest, nullptr), ZX_OK);
    ASSERT_EQ(cache.Evict(node), ZX_OK);
    node->SetCache(true);
  }
  ASSERT_EQ(ZX_ERR_NOT_FOUND, cache.Lookup(digest, nullptr));
}

// This helper function only operates correctly when a single node is open in the cache.
void CheckExistsAloneInOpenCache(BlobCache* cache, void* node_ptr) {
  ASSERT_TRUE(cache);
  int node_count = 0;
  cache->ForAllOpenNodes([&node_count, &node_ptr](fbl::RefPtr<CacheNode> node) {
    node_count++;
    ZX_ASSERT(node.get() == node_ptr);
    return ZX_OK;
  });
  ASSERT_EQ(1, node_count);
}

TEST(BlobCacheTest, CacheAfterRecycle) {
  BlobCache cache;
  Digest digest = GenerateDigest(0);
  void* node_ptr = nullptr;

  // Add a node to the cache.
  {
    auto node = fbl::MakeRefCounted<TestNode>(digest, &cache);
    node_ptr = node.get();
    ASSERT_EQ(cache.Add(node), ZX_OK);
    ASSERT_EQ(cache.Lookup(digest, nullptr), ZX_OK);

    // Observe the node is in the set of open nodes.
    CheckExistsAloneInOpenCache(&cache, node_ptr);
  }

  // Observe the node is in no longer in the set of open nodes, now that it has
  // run out of strong references.
  CheckNothingOpenHelper(&cache);

  // Observe that although the node in in the "closed set", it still exists in the cache,
  // and can be re-acquired.
  ASSERT_EQ(cache.Lookup(digest, nullptr), ZX_OK);

  // Letting the node go out of scope puts it back in the cache.
  {
    fbl::RefPtr<CacheNode> node;
    ASSERT_EQ(cache.Lookup(digest, &node), ZX_OK);
    ASSERT_EQ(node_ptr, node.get());
    CheckExistsAloneInOpenCache(&cache, node_ptr);
  }
  ASSERT_EQ(cache.Lookup(digest, nullptr), ZX_OK);

  // However, if we stop caching the node, it will be deleted when all references
  // go out of scope.
  {
    fbl::RefPtr<CacheNode> cache_node;
    ASSERT_EQ(cache.Lookup(digest, &cache_node), ZX_OK);
    auto vnode = fbl::RefPtr<TestNode>::Downcast(std::move(cache_node));
    ASSERT_EQ(cache.Evict(vnode), ZX_OK);
  }
  ASSERT_EQ(ZX_ERR_NOT_FOUND, cache.Lookup(digest, nullptr));
}

TEST(BlobCacheTest, ResetClosed) {
  BlobCache cache;
  // Create a node which exists in the closed cache.
  Digest digest = GenerateDigest(0);
  {
    auto node = fbl::MakeRefCounted<TestNode>(digest, &cache);
    ASSERT_EQ(cache.Add(node), ZX_OK);
  }
  ASSERT_EQ(cache.Lookup(digest, nullptr), ZX_OK);

  // After resetting, the node should no longer exist.
  cache.Reset();
  ASSERT_EQ(ZX_ERR_NOT_FOUND, cache.Lookup(digest, nullptr));
}

TEST(BlobCacheTest, ResetOpen) {
  BlobCache cache;
  // Create a node which exists in the open cache.
  Digest digest = GenerateDigest(0);
  auto node = fbl::MakeRefCounted<TestNode>(digest, &cache);
  node->SetHighMemory();
  ASSERT_EQ(cache.Add(node), ZX_OK);

  // After resetting, the node should no longer exist.
  cache.Reset();
  ASSERT_EQ(ZX_ERR_NOT_FOUND, cache.Lookup(digest, nullptr));
  ASSERT_TRUE(node->UsingMemory());
}

TEST(BlobCacheTest, Destructor) {
  auto cache = std::make_unique<BlobCache>();
  Digest open_digest = GenerateDigest(0);
  fbl::RefPtr<TestNode> open_node = fbl::MakeRefCounted<TestNode>(open_digest, cache.get());
  ASSERT_EQ(cache->Add(open_node), ZX_OK);
  if constexpr (ZX_DEBUG_ASSERT_IMPLEMENTED) {
    // Destroying the cache with a node that's still open will trip a debug assert.
    ASSERT_DEATH({ cache.reset(); }, "");
  }
}

TEST(BlobCacheTest, ForAllOpenNodes) {
  BlobCache cache;

  // Add a bunch of open nodes to the cache.
  fbl::RefPtr<TestNode> open_nodes[10];
  for (size_t i = 0; i < std::size(open_nodes); i++) {
    open_nodes[i] = fbl::MakeRefCounted<TestNode>(GenerateDigest(i), &cache);
    ASSERT_EQ(cache.Add(open_nodes[i]), ZX_OK);
  }

  // For fun, add some nodes to the cache which will become non-open:
  // One which runs out of strong references, and another which is evicted.
  {
    auto node = fbl::MakeRefCounted<TestNode>(GenerateDigest(0xDEAD), &cache);
    ASSERT_EQ(cache.Add(node), ZX_OK);
  }
  auto node = fbl::MakeRefCounted<TestNode>(GenerateDigest(0xBEEF), &cache);
  ASSERT_EQ(cache.Add(node), ZX_OK);
  ASSERT_EQ(cache.Evict(node), ZX_OK);

  // Double check that the nodes which should be open are open, and that the nodes
  // which aren't open aren't visible.
  size_t node_index = 0;
  cache.ForAllOpenNodes([&open_nodes, &node_index](fbl::RefPtr<CacheNode> node) {
    ZX_ASSERT(node_index < std::size(open_nodes));
    for (size_t i = 0; i < std::size(open_nodes); i++) {
      // We should be able to find this node in the set of open nodes -- but only once.
      if (open_nodes[i] && open_nodes[i].get() == node.get()) {
        open_nodes[i] = nullptr;
        node_index++;
        return ZX_OK;
      }
    }
    ZX_ASSERT_MSG(false, "Found open node not contained in expected open set");
  });
  ASSERT_EQ(std::size(open_nodes), node_index);
}

TEST(BlobCacheTest, CachePolicyEvictImmediately) {
  BlobCache cache;
  Digest digest = GenerateDigest(0);

  cache.SetCachePolicy(CachePolicy::EvictImmediately);
  {
    auto node = fbl::MakeRefCounted<TestNode>(digest, &cache);
    node->SetHighMemory();
    ASSERT_EQ(cache.Add(node), ZX_OK);
    ASSERT_TRUE(node->UsingMemory());
  }

  fbl::RefPtr<CacheNode> cache_node;
  ASSERT_EQ(cache.Lookup(digest, &cache_node), ZX_OK);
  auto node = fbl::RefPtr<TestNode>::Downcast(std::move(cache_node));
  ASSERT_FALSE(node->UsingMemory());
}

TEST(BlobCacheTest, CachePolicyNeverEvict) {
  BlobCache cache;
  Digest digest = GenerateDigest(0);

  cache.SetCachePolicy(CachePolicy::NeverEvict);
  {
    auto node = fbl::MakeRefCounted<TestNode>(digest, &cache);
    node->SetHighMemory();
    ASSERT_EQ(cache.Add(node), ZX_OK);
    ASSERT_TRUE(node->UsingMemory());
  }

  fbl::RefPtr<CacheNode> cache_node;
  ASSERT_EQ(cache.Lookup(digest, &cache_node), ZX_OK);
  auto node = fbl::RefPtr<TestNode>::Downcast(std::move(cache_node));
  ASSERT_TRUE(node->UsingMemory());
}

TEST(BlobCacheTest, CachePolicyOverrideSettingsRespected) {
  BlobCache cache;
  Digest digest = GenerateDigest(0);

  cache.SetCachePolicy(CachePolicy::NeverEvict);
  {
    auto node = fbl::MakeRefCounted<TestNode>(digest, &cache);
    node->SetHighMemory();
    node->set_overridden_cache_policy(CachePolicy::EvictImmediately);
    ASSERT_EQ(cache.Add(node), ZX_OK);
    ASSERT_TRUE(node->UsingMemory());
  }

  fbl::RefPtr<CacheNode> cache_node;
  ASSERT_EQ(cache.Lookup(digest, &cache_node), ZX_OK);
  auto node = fbl::RefPtr<TestNode>::Downcast(std::move(cache_node));
  // Was evicted
  ASSERT_FALSE(node->UsingMemory());
}

}  // namespace
}  // namespace blobfs
