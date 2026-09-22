use std::path::PathBuf;

use arcscan_topology::replay::{compare_expected, ReplayFixture};

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| {
            "Usage: cargo run --manifest-path src-tauri/topology-engine/Cargo.toml --bin replay-topology -- path/to/fixture.json".to_string()
        })?;
    let json = std::fs::read_to_string(&path)
        .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    let fixture = ReplayFixture::parse(&json).map_err(|error| error.to_string())?;
    let result = fixture.replay().map_err(|error| error.to_string())?;
    let diff = compare_expected(&fixture, &result);
    if !diff.is_empty() {
        return Err(format!(
            "Topology replay expectations failed for {}:\n\n{diff}",
            path.display()
        ));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&result)
            .map_err(|error| format!("Could not serialize replay result: {error}"))?
    );
    Ok(())
}
