// The gix call from the report: merge `side` into `main`, then print the merged tree and the conflicts.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let repo = gix::open(".")?;
    let (ours, theirs) = (repo.rev_parse_single("main")?, repo.rev_parse_single("side")?);
    let mut out = repo.merge_commits(ours, theirs, Default::default(), repo.tree_merge_options()?.into())?;
    let unresolved = out.tree_merge.has_unresolved_conflicts(Default::default());
    let resolutions: Vec<_> = out.tree_merge.conflicts.iter().map(|c| format!("{:?}", c.resolution)).collect();
    println!("tree {} unresolved_conflicts {unresolved} {}", out.tree_merge.tree.write()?, resolutions.join(";"));
    Ok(())
}
