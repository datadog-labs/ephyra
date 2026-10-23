// Usage: status <repo-dir> ; prints each index/worktree status item as `<summary> <path>`.
fn main() {
    let repo = gix::open(std::env::args().nth(1).expect("repo dir")).expect("open repo");
    for item in repo
        .status(gix::progress::Discard)
        .expect("status")
        .into_index_worktree_iter(None)
        .expect("iter")
    {
        let item = item.expect("item");
        if let Some(summary) = item.summary() {
            println!("{summary:?} {}", item.rela_path());
        }
    }
}
