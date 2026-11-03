//! Background work that must never slow the game down.
//!
//! The simulations of the turn plan and of the fight odds run again each time the board, the gold or the
//! tier changes (buy, sell, level up). They used to run at normal priority on several cores (all of them for
//! a fight), so on a small CPU Hearthstone could miss a few frames right when the player acted.
//!
//! - `lower_priority()` puts the calling thread below Hearthstone's priority (Windows; nothing elsewhere):
//!   the game always gets the CPU first, our work fills the gaps.
//! - `sim_threads()`: the simulator's own thread pool uses half the logical cores at most, so the game
//!   always has free cores too.

/// Number of simulator threads for a CPU with `cores` logical cores: half, at least one.
pub fn sim_threads_for(cores: usize) -> usize {
    (cores / 2).max(1)
}

/// Number of simulator threads on this computer.
pub fn sim_threads() -> usize {
    sim_threads_for(
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(2),
    )
}

/// Lowers the priority of the calling thread (below normal on Windows).
#[cfg(windows)]
pub fn lower_priority() {
    use windows_sys::Win32::System::Threading::{
        GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
    };
    // SAFETY: GetCurrentThread returns a pseudo handle of this thread, always valid; nothing to free.
    unsafe {
        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
    }
}

/// Lowers the priority of the calling thread (below normal on Windows).
#[cfg(not(windows))]
pub fn lower_priority() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_the_cores_at_least_one() {
        assert_eq!(sim_threads_for(1), 1);
        assert_eq!(sim_threads_for(2), 1);
        assert_eq!(sim_threads_for(4), 2);
        assert_eq!(sim_threads_for(8), 4);
        assert_eq!(sim_threads_for(16), 8);
        assert!(sim_threads() >= 1);
    }
}
