//! The `ephyra` command, as a library: [`serve`] runs properties into one
//! artifact directory and serves them on a local page, and [`check`] reads
//! the workspace's `#[ephyra::test]` and `#[ephyra::generator]`
//! functions from source and runs the checks of its generators and
//! strategies.

pub mod check;
mod index;
pub mod serve;

/// The guide `ephyra guide` prints, for whoever writes generators and
/// strategies. Its examples are tested here.
pub const GUIDE: &str = include_str!("../GUIDE.md");

#[cfg(doctest)]
#[doc = include_str!("../GUIDE.md")]
struct Guide;
