//! A line diff for showing what replacing the policy would change.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Same,
    Removed,
    Added,
}

/// A unified-style diff with `context` lines around each change, or `None` when the texts match.
pub fn render(old: &str, new: &str, context: usize) -> Option<String> {
    let old: Vec<&str> = old.lines().collect();
    let new: Vec<&str> = new.lines().collect();
    let ops = script(&old, &new);
    if ops.iter().all(|(op, _)| *op == Op::Same) {
        return None;
    }
    let mut show = vec![false; ops.len()];
    for (index, (op, _)) in ops.iter().enumerate() {
        if *op != Op::Same {
            for near in index.saturating_sub(context)..(index + context + 1).min(ops.len()) {
                show[near] = true;
            }
        }
    }
    let mut out = String::new();
    let mut hidden = false;
    for (index, (op, line)) in ops.iter().enumerate() {
        if !show[index] {
            hidden = true;
            continue;
        }
        if hidden && !out.is_empty() {
            out.push_str("  …\n");
        }
        hidden = false;
        let mark = match op {
            Op::Same => ' ',
            Op::Removed => '-',
            Op::Added => '+',
        };
        out.push(mark);
        out.push(' ');
        out.push_str(line);
        out.push('\n');
    }
    Some(out)
}

/// The shortest edit script between the two line lists (longest common subsequence).
fn script<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<(Op, &'a str)> {
    // Trim the shared ends first; policy edits are usually small and local.
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..].iter().rev().zip(new[prefix..].iter().rev()).take_while(|(a, b)| a == b).count();
    let (a, b) = (&old[prefix..old.len() - suffix], &new[prefix..new.len() - suffix]);
    let mut table = vec![vec![0u32; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            table[i][j] = if a[i] == b[j] { table[i + 1][j + 1] + 1 } else { table[i + 1][j].max(table[i][j + 1]) };
        }
    }
    let mut ops: Vec<(Op, &str)> = old[..prefix].iter().map(|line| (Op::Same, *line)).collect();
    let (mut i, mut j) = (0, 0);
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            ops.push((Op::Same, a[i]));
            i += 1;
            j += 1;
        } else if i < a.len() && (j == b.len() || table[i + 1][j] >= table[i][j + 1]) {
            ops.push((Op::Removed, a[i]));
            i += 1;
        } else {
            ops.push((Op::Added, b[j]));
            j += 1;
        }
    }
    ops.extend(old[old.len() - suffix..].iter().map(|line| (Op::Same, *line)));
    ops
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_texts_have_no_diff() {
        assert_eq!(render("a\nb\n", "a\nb\n", 2), None);
        assert_eq!(render("", "", 2), None);
    }

    #[test]
    fn shows_additions_and_removals_with_context() {
        let old = "1\n2\n3\n4\n5\n6\n7\n8\n";
        let new = "1\n2\n3\n4\nfive\n6\n7\n8\n";
        assert_eq!(render(old, new, 1).unwrap(), "  4\n- 5\n+ five\n  6\n");
    }

    #[test]
    fn marks_skipped_stretches_between_changes() {
        let old = "a\n1\n2\n3\n4\n5\nb\n";
        let new = "A\n1\n2\n3\n4\n5\nB\n";
        let diff = render(old, new, 1).unwrap();
        assert_eq!(diff, "- a\n+ A\n  1\n  …\n  5\n- b\n+ B\n");
    }

    #[test]
    fn handles_insertions_and_deletions_at_the_ends() {
        assert_eq!(render("b\n", "a\nb\nc\n", 0).unwrap(), "+ a\n  …\n+ c\n");
        assert_eq!(render("a\nb\nc\n", "b\n", 0).unwrap(), "- a\n  …\n- c\n");
    }
}
