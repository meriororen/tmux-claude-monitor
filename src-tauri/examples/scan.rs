//! Manual check: one poller tick against the live system, printed as text.
//! Run: cargo run --example scan

use tmux_claude_monitor_lib::{poller, procs, tmux};

fn main() {
    let bin = tmux::resolve_tmux_bin().expect("tmux not found");
    let ps = procs::snapshot_ps();
    let mut cache = std::collections::HashMap::new();
    let cards = poller::build_cards_for_example(&bin, &ps, &mut cache, 30);
    println!("{} claude session(s):", cards.len());
    for c in &cards {
        println!(
            "  [{:?}] {} ({}:{}.{}) idle={}s\n      {}",
            c.state, c.title, c.tmux_session, c.window_index, c.pane_index, c.idle_secs, c.summary
        );
    }
}
