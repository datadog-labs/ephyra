// topo <repo> <tip-id> <end-id> [first-parent|all] [date|topo]: prints what gix's topo walk returns
// for `git rev-list --date-order|--topo-order [--first-parent] <tip> ^<end>`, one id per line.
use gix::traverse::commit::{topo, Parents};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let repo = gix::open(&a[1]).expect("open repo");
    let tip = gix::ObjectId::from_hex(a[2].as_bytes()).expect("tip id");
    let end = gix::ObjectId::from_hex(a[3].as_bytes()).expect("end id");
    let parents = if a.get(4).map_or(true, |p| p == "first-parent") { Parents::First } else { Parents::All };
    let walk = topo::Builder::from_iters(&repo.objects, [tip], Some([end]))
        .sorting(if a.get(5).map_or(false, |s| s == "topo") { topo::Sorting::TopoOrder } else { topo::Sorting::DateOrder })
        .parents(parents)
        .build()
        .expect("build");
    for info in walk {
        println!("{}", info.expect("walk").id);
    }
}
