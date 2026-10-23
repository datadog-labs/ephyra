// The gix call from the report: merge `side` into `main`, then print the merged tree and the conflict state.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let repo = gix::open(".")?;
    let (ours, theirs) = (repo.rev_parse_single("main")?, repo.rev_parse_single("side")?);
    let mut out = repo.merge_commits(ours, theirs, Default::default(), repo.tree_merge_options()?.into())?;
    let unresolved = out.tree_merge.has_unresolved_conflicts(Default::default());
    println!("tree {} unresolved_conflicts {unresolved}", out.tree_merge.tree.write()?);
    Ok(())
}
