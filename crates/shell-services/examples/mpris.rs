//! Prints the media players running and what they're playing: `cargo run --example mpris`.

use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    for player in shell_services::mpris::query()? {
        println!("{player:#?}\nrank {}, at {:?}", player.rank(), player.position(Instant::now()));
    }
    Ok(())
}
