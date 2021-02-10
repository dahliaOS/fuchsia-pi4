use {
    super::SimpleAllocator,
    crate::object_store::{allocator::Allocator, Log},
    anyhow::Error,
    std::sync::Arc,
};

#[test]
fn test_allocate_reserves() -> Result<(), Error> {
    let log = Arc::new(Log::new());
    let allocator = SimpleAllocator::new(&log);
    let allocation1 = allocator.allocate(1, 0, 0..512)?;
    let allocation2 = allocator.allocate(1, 0, 0..512)?;
    assert!(allocation2.start >= allocation1.end || allocation2.end <= allocation1.start);
    Ok(())
}
