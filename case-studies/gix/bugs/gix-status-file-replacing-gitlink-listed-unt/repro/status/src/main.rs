// Usage: status <repo-dir> ; prints each worktree status item as `<summary> <path>`.
fn main() {
    let repo = gix::open(std::env::args().nth(1).expect("repo dir")).expect("open repo");
    let mut untracked = false;
    for item in repo
        .status(gix::progress::Discard)
        .expect("status")
        .into_index_worktree_iter(None)
        .expect("iter")
    {
        let item = item.expect("item");
        if let Some(summary) = item.summary() {
            untracked |= matches!(summary, gix::status::index_worktree::iter::Summary::Added);
            println!("{summary:?} {}", item.rela_path());
        }
    }
    std::process::exit(untracked as i32);
}
