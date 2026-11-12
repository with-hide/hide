//! `memory::return_freed` answers on every system this repository builds
//! for; what it saves a resident process is measured where one runs
//! (`hide-holder/tests/it/memory.rs`). `memory::pressure` reads the live
//! level on every CI lane; a system without the signal is an error, which
//! the module's own tests drive with the text and levels it rejects.

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

/// The level reads on macOS, Linux and Windows, and reading it again
/// answers again: nothing is held open between reads.
#[test]
fn the_machines_memory_pressure_reads_as_a_level() {
    for _ in 0..3 {
        let level = hide_platform::memory::pressure()
            .expect("this system reports its memory pressure");
        eprintln!("memory pressure: {}", level.code());
    }
}
