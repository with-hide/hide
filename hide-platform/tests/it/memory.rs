//! `memory::return_freed` answers on every system this repository builds
//! for; what it saves a resident process is measured where one runs
//! (`hide-holder/tests/it/memory.rs`).

#[test]
fn freed_memory_can_be_given_back() {
    let blocks: Vec<Vec<u8>> = (0..1024)
        .map(|index| vec![index as u8; 16 * 1024])
        .collect();
    drop(blocks);
    hide_platform::memory::return_freed().unwrap();
    // Asked again with nothing freed since, it still answers.
    hide_platform::memory::return_freed().unwrap();
}
