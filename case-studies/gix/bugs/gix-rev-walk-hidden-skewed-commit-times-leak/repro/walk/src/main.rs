// Usage: walk <rev>... ^<rev>...   (run inside a repository)
// Prints the commits of `repo.rev_walk(tips).with_hidden(hidden)`, one id per line,
// like `git rev-list <rev>... ^<rev>...`.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let repo = gix::discover(".")?;
    let (mut tips, mut hidden) = (Vec::new(), Vec::new());
    for arg in std::env::args().skip(1) {
        match arg.strip_prefix('^') {
            Some(rev) => hidden.push(repo.rev_parse_single(rev)?.detach()),
            None => tips.push(repo.rev_parse_single(arg.as_str())?.detach()),
        }
    }
    for info in repo.rev_walk(tips).with_hidden(hidden).all()? {
        println!("{}", info?.id);
    }
    Ok(())
}
