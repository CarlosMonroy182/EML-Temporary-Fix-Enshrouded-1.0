use linuxtest::native::spawner::drop_stale_world;

#[test]
fn a_new_client_world_long_after_the_hosted_one_means_the_host_is_gone() {
    assert!(drop_stale_world(0x20, 0x10, 400_000, 100_000));
}

#[test]
fn a_client_world_next_to_a_fresh_hosted_one_keeps_it() {
    assert!(!drop_stale_world(0x20, 0x10, 400_000, 380_000));
    assert!(!drop_stale_world(0x20, 0, 30_000, 12_000));
}

#[test]
fn the_same_client_world_never_drops_it() {
    assert!(!drop_stale_world(0x20, 0x20, 900_000, 1_000));
}
