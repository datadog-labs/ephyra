// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Programs as trees of lines: a node is one line and its indented block.
//! Shrinking works on this shape only (drop a node, hoist a block), so
//! candidates may be invalid Python; the oracle rejects those because they
//! fail differently.

use std::ops::ControlFlow;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Tree {
    pub line: String,
    pub kids: Vec<Tree>,
    /// Decorator-style siblings printed before this node (kept with it).
    pub pre: Vec<String>,
}

pub fn node(line: impl Into<String>) -> Tree {
    Tree { line: line.into(), kids: vec![], pre: vec![] }
}

impl Tree {
    pub fn root(kids: Vec<Tree>) -> Tree {
        Tree { line: String::new(), kids, pre: vec![] }
    }

    pub fn with(mut self, kids: Vec<Tree>) -> Tree {
        self.kids = kids;
        self
    }

    /// `self` is a decorator line for `next`.
    pub fn then(self, mut next: Tree) -> Tree {
        next.pre.insert(0, self.line);
        next
    }

    pub fn source(&self) -> String {
        let mut out = String::new();
        for k in &self.kids {
            k.write(0, &mut out);
        }
        out
    }

    fn write(&self, ind: usize, out: &mut String) {
        for p in &self.pre {
            out.push_str(&"    ".repeat(ind));
            out.push_str(p);
            out.push('\n');
        }
        out.push_str(&"    ".repeat(ind));
        out.push_str(&self.line);
        out.push('\n');
        for k in &self.kids {
            k.write(ind + 1, out);
        }
    }

    pub fn size(&self) -> usize {
        1 + self.kids.iter().map(Tree::size).sum::<usize>()
    }

    /// Paths (child index sequences) of every node, pre-order, outermost first.
    fn paths(&self, prefix: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
        for (i, k) in self.kids.iter().enumerate() {
            prefix.push(i);
            out.push(prefix.clone());
            k.paths(prefix, out);
            prefix.pop();
        }
    }

    fn at_mut(&mut self, path: &[usize]) -> &mut Tree {
        let mut t = self;
        for &i in path {
            t = &mut t.kids[i];
        }
        t
    }

    fn at(&self, path: &[usize]) -> &Tree {
        let mut t = self;
        for &i in path {
            t = &t.kids[i];
        }
        t
    }

    /// Shrink candidates, smaller first: drop runs of siblings, drop one node,
    /// replace a node by its block, empty a block to `pass`.
    pub fn shrink(&self, visit: &mut dyn FnMut(Tree) -> ControlFlow<()>) -> ControlFlow<()> {
        let mut paths = Vec::new();
        self.paths(&mut Vec::new(), &mut paths);
        // Halves of top-level and block-level sibling runs.
        let mut parents: Vec<Vec<usize>> = vec![vec![]];
        parents.extend(paths.iter().filter(|p| !self.at(p).kids.is_empty()).cloned());
        for parent in &parents {
            let n = self.at(parent).kids.len();
            let mut chunk = n / 2;
            while chunk >= 2 {
                let mut start = 0;
                while start + chunk <= n {
                    let mut t = self.clone();
                    let kids = &mut t.at_mut(parent).kids;
                    kids.drain(start..start + chunk);
                    if kids.is_empty() && !parent.is_empty() {
                        kids.push(node("pass"));
                    }
                    visit(t)?;
                    start += chunk;
                }
                chunk /= 2;
            }
        }
        for p in &paths {
            let (last, parent) = p.split_last().unwrap();
            let mut t = self.clone();
            let kids = &mut t.at_mut(parent).kids;
            if kids[*last].line == "pass" {
                continue;
            }
            kids.remove(*last);
            if kids.is_empty() && !parent.is_empty() {
                kids.push(node("pass"));
            }
            visit(t)?;
        }
        for p in &paths {
            let n = self.at(p);
            if n.kids.is_empty() {
                continue;
            }
            let (last, parent) = p.split_last().unwrap();
            let mut t = self.clone();
            let block = t.at(p).kids.clone();
            let kids = &mut t.at_mut(parent).kids;
            kids.splice(*last..=*last, block);
            visit(t)?;
            if !(n.kids.len() == 1 && n.kids[0].line == "pass") {
                let mut t = self.clone();
                t.at_mut(p).kids = vec![node("pass")];
                visit(t)?;
            }
        }
        ControlFlow::Continue(())
    }
}
