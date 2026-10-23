// Usage: status <repo-dir> ; prints what gix thinks HEAD is, then runs a full status.
fn chain(err: &dyn std::error::Error) -> String {
    let mut msg = err.to_string();
    let mut src = err.source();
    while let Some(s) = src {
        msg.push_str(&format!(": {s}"));
        src = s.source();
    }
    msg
}

fn main() {
    let repo = gix::open(std::env::args().nth(1).expect("repo dir")).expect("open repo");
    let head = repo.head().expect("head");
    println!("head().is_unborn(): {}", head.is_unborn());
    let mut failed = false;
    match repo.head_tree_id_or_empty() {
        Ok(id) => println!("head_tree_id_or_empty(): {id}"),
        Err(err) => {
            failed = true;
            println!("head_tree_id_or_empty(): error: {}", chain(&err))
        }
    }
    match repo.is_dirty() {
        Ok(d) => println!("is_dirty(): {d}"),
        Err(err) => {
            failed = true;
            println!("is_dirty(): error: {}", chain(&err))
        }
    }
    let status = repo
        .status(gix::progress::Discard)
        .and_then(|s| s.into_iter(None))
        .map_err(|e| chain(&e))
        .and_then(|iter| {
            iter.map(|item| item.map(|i| format!("{:?}", i.location())).map_err(|e| chain(&e)))
                .collect::<Result<Vec<_>, _>>()
        });
    match status {
        Ok(items) => println!("status(): ok, {} item(s) {:?}", items.len(), items),
        Err(err) => {
            failed = true;
            println!("status(): error: {err}")
        }
    }
    std::process::exit(failed as i32);
}
