use super::char_edits::{to_char_edits, TextEdit};
use super::line_diff::{apply_hunks, line_diff, split_lines, LineHunk};
use std::cmp::Ordering;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct MergeResult {
    pub edits: Vec<TextEdit>,
    pub displaced: Vec<String>,
    pub coarse: bool,
}

struct SidedHunk {
    hunk: LineHunk,
    external: bool,
}

fn overlaps(left: &LineHunk, right: &LineHunk) -> bool {
    left.from < right.to && right.from < left.to
}

fn contains(region: &LineHunk, hunk: &LineHunk) -> bool {
    hunk.from >= region.from && hunk.to <= region.to
}

fn by_position(left: &LineHunk, right: &LineHunk) -> Ordering {
    left.from.cmp(&right.from).then(left.to.cmp(&right.to))
}

fn regions(mut hunks: Vec<SidedHunk>) -> Vec<Vec<SidedHunk>> {
    hunks.sort_by(|left, right| by_position(&left.hunk, &right.hunk));
    let mut grouped: Vec<Vec<SidedHunk>> = Vec::new();
    let mut reach = 0;
    for sided in hunks {
        match grouped.last_mut() {
            Some(region) if sided.hunk.from < reach => {
                reach = reach.max(sided.hunk.to);
                region.push(sided);
            }
            _ => {
                reach = sided.hunk.to;
                grouped.push(vec![sided]);
            }
        }
    }
    grouped
}

fn side(region: &[SidedHunk], external: bool) -> Vec<LineHunk> {
    region.iter().filter(|sided| sided.external == external).map(|sided| sided.hunk.clone()).collect()
}

pub fn merge_external_change(base: &str, disk: &str, current: &str) -> MergeResult {
    if disk == current {
        return MergeResult::default();
    }
    let base_lines = split_lines(base);
    let disk_diff = line_diff(&base_lines, &split_lines(disk));
    if disk_diff.hunks.is_empty() {
        return MergeResult::default();
    }
    let current_lines = split_lines(current);
    let local_diff = line_diff(&base_lines, &current_lines);

    let sided = disk_diff
        .hunks
        .iter()
        .map(|hunk| SidedHunk { hunk: hunk.clone(), external: true })
        .chain(local_diff.hunks.iter().map(|hunk| SidedHunk { hunk: hunk.clone(), external: false }))
        .collect();

    let mut displaced = Vec::new();
    let mut resolved = Vec::new();
    for region in regions(sided) {
        let external = side(&region, true);
        if external.is_empty() {
            continue;
        }
        let from = region.iter().map(|sided| sided.hunk.from).min().unwrap_or(0);
        let to = region.iter().map(|sided| sided.hunk.to).max().unwrap_or(0);
        let local = side(&region, false);
        if !local.is_empty() {
            displaced.extend(apply_hunks(&base_lines, &local, from, to));
        }
        resolved.push(LineHunk { from, to, lines: apply_hunks(&base_lines, &external, from, to) });
    }

    let untouched_local = local_diff
        .hunks
        .iter()
        .filter(|hunk| !resolved.iter().any(|region| overlaps(region, hunk) || contains(region, hunk)))
        .cloned();
    let mut kept: Vec<LineHunk> = resolved.iter().cloned().chain(untouched_local).collect();
    kept.sort_by(by_position);
    let merged = apply_hunks(&base_lines, &kept, 0, base_lines.len());
    let merged: Vec<&str> = merged.iter().map(String::as_str).collect();

    MergeResult {
        edits: to_char_edits(current, &current_lines, &line_diff(&current_lines, &merged).hunks),
        displaced: displaced.into_iter().filter(|line| !line.trim().is_empty()).collect(),
        coarse: disk_diff.coarse || local_diff.coarse,
    }
}
