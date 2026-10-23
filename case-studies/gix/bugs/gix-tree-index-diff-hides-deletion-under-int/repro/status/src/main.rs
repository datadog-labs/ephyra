// Prints the HEAD-vs-index part of `Repository::status()`, one line per change.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let repo = gix::open(std::env::args().nth(1).unwrap_or(".".into()))?;
    for item in repo.status(gix::progress::Discard)?.into_iter(None)? {
        if let gix::status::Item::TreeIndex(change) = item? {
            let kind = match change {
                gix::diff::index::Change::Addition { .. } => "A",
                gix::diff::index::Change::Deletion { .. } => "D",
                gix::diff::index::Change::Modification { .. } => "M",
                gix::diff::index::Change::Rewrite { .. } => "R",
            };
            println!("{kind} {}", change.location());
        }
    }
    Ok(())
}
