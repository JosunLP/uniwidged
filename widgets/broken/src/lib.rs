//! A widget that hangs, every time, from a fresh instance.
//!
//! The conformance widget can be *told* to hang, which turns out to test
//! something different and rather more cheerful: a fresh instance does not
//! carry the state that made the old one hang, so restarting cures it. That is
//! the behaviour a user wants and it is worth having a test for — but it is
//! not the behaviour that has to be contained.
//!
//! This is the one that has to be contained. Its `view` never returns, on
//! every instance, so restarting cannot help and the host has to stop trying.

use uniwidged_guest::prelude::*;

#[derive(Default)]
pub struct Broken;

impl Widget for Broken {
    fn view(&mut self, _cx: &Cx) -> View {
        // Unbounded, with no allocation and no host call in it, so nothing but
        // wall-clock time can stop it. `black_box` keeps the optimiser from
        // noticing the loop does nothing and removing it.
        loop {
            std::hint::black_box(0u64);
        }
    }
}

uniwidged_guest::export!(Broken);
