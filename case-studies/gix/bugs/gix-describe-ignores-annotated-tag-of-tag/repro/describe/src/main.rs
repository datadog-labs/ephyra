// The one-line gix call from the report: gix's default `describe()` (annotated tags only) for HEAD.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("{:?}", gix::open(".")?.head_commit()?.describe().try_format()?.map(|f| f.to_string()));
    Ok(())
}
