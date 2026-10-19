//! Pure layout helpers: side-by-side packing of overlapping timed events in
//! a day column, and lane packing for bars that span days (all-day row,
//! month view).

/// Where an event sits among the overlapping events of its cluster.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ColBox {
    /// Column index inside the cluster (0-based).
    pub col: usize,
    /// How many columns it stretches across (free columns to its right).
    pub span: usize,
    /// Columns in the cluster.
    pub cols: usize,
    /// Which cluster (group of transitively overlapping segments) it is in,
    /// numbered from 0 in time order.
    pub cluster: usize,
}

/// Narrowest an event block may get before the rest of its cluster folds
/// into a "+N" chip.
pub const MIN_EVENT_W: f64 = 28.0;

/// Columns of a cluster that still fit `avail` pixels with each at least
/// `MIN_EVENT_W` wide, leaving one column for the "+N" chip when the
/// cluster doesn't fit. `None` when everything fits.
pub fn visible_columns(cols: usize, avail: f64) -> Option<usize> {
    if avail <= 0.0 || avail / cols.max(1) as f64 >= MIN_EVENT_W {
        return None;
    }
    let fit = (avail / MIN_EVENT_W).floor() as usize;
    Some(fit.saturating_sub(1).max(1))
}

/// Lay out timed segments of one day side by side (Outlook's algorithm):
/// sort by start (longer first on ties), group transitively overlapping
/// segments into clusters, put each segment into the first column whose last
/// segment has ended, then let segments stretch over columns to their right
/// that stay free for their whole span.
///
/// `items` are `(start, end)` in any unit (minutes); a segment shorter than
/// `min_len` is treated as `min_len` long so short events don't collide
/// visually. Results are in input order.
pub fn pack_columns(items: &[(i64, i64)], min_len: i64) -> Vec<ColBox> {
    let n = items.len();
    let vend = |i: usize| items[i].1.max(items[i].0 + min_len);
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| items[a].0.cmp(&items[b].0).then(vend(b).cmp(&vend(a))).then(a.cmp(&b)));

    let mut out = vec![ColBox::default(); n];
    let mut cluster: Vec<usize> = Vec::new();
    let mut col_ends: Vec<i64> = Vec::new();
    let mut cluster_end = i64::MIN;
    let mut cluster_no = 0;

    let finish = |cluster: &mut Vec<usize>, col_ends: &mut Vec<i64>, out: &mut Vec<ColBox>, no: &mut usize| {
        let cols = col_ends.len().max(1);
        for &i in cluster.iter() {
            out[i].cluster = *no;
            let col = out[i].col;
            let mut span = 1;
            for c in col + 1..cols {
                let blocked = cluster.iter().any(|&j| out[j].col == c && items[j].0 < vend(i) && vend(j) > items[i].0);
                if blocked {
                    break;
                }
                span += 1;
            }
            out[i].span = span;
            out[i].cols = cols;
        }
        *no += 1;
        cluster.clear();
        col_ends.clear();
    };

    for &i in &order {
        let (start, _) = items[i];
        if !cluster.is_empty() && start >= cluster_end {
            finish(&mut cluster, &mut col_ends, &mut out, &mut cluster_no);
            cluster_end = i64::MIN;
        }
        let col = match col_ends.iter().position(|&e| e <= start) {
            Some(c) => {
                col_ends[c] = vend(i);
                c
            }
            None => {
                col_ends.push(vend(i));
                col_ends.len() - 1
            }
        };
        out[i].col = col;
        cluster.push(i);
        cluster_end = cluster_end.max(vend(i));
    }
    if !cluster.is_empty() {
        finish(&mut cluster, &mut col_ends, &mut out, &mut cluster_no);
    }
    out
}

/// Greedy lane packing for bars covering day indices `first..=last`:
/// sorted by first day (longer first), each bar takes the lowest lane that is
/// free on all its days. Returns the lane of each item, in input order.
pub fn pack_lanes(items: &[(usize, usize)]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|&a, &b| {
        items[a].0.cmp(&items[b].0).then((items[b].1 - items[b].0).cmp(&(items[a].1 - items[a].0))).then(a.cmp(&b))
    });
    let mut lane_last: Vec<usize> = Vec::new();
    let mut out = vec![0; items.len()];
    for i in order {
        let (first, last) = items[i];
        let lane = match lane_last.iter().position(|&l| l < first) {
            Some(l) => l,
            None => {
                lane_last.push(0);
                lane_last.len() - 1
            }
        };
        lane_last[lane] = last;
        out[i] = lane;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cb(col: usize, span: usize, cols: usize) -> ColBox {
        ColBox { col, span, cols, cluster: 0 }
    }

    /// Boxes without the cluster number, for comparing layouts.
    fn boxes(items: &[(i64, i64)], min_len: i64) -> Vec<ColBox> {
        pack_columns(items, min_len).into_iter().map(|b| ColBox { cluster: 0, ..b }).collect()
    }

    #[test]
    fn separate_events_take_full_width() {
        assert_eq!(boxes(&[(0, 60), (60, 120)], 30), vec![cb(0, 1, 1), cb(0, 1, 1)]);
        assert!(pack_columns(&[], 30).is_empty());
        // Separate clusters are numbered in time order.
        let got = pack_columns(&[(60, 120), (0, 30), (10, 40)], 30);
        assert_eq!(got.iter().map(|b| b.cluster).collect::<Vec<_>>(), vec![1, 0, 0]);
    }

    #[test]
    fn overlapping_triple_gets_three_columns() {
        // 11:00–12:00, 11:15–12:15, 11:30–12:30 (input order shuffled).
        let got = boxes(&[(690, 750), (660, 720), (675, 735)], 30);
        assert_eq!(got, vec![cb(2, 1, 3), cb(0, 1, 3), cb(1, 1, 3)]);
    }

    #[test]
    fn chains_reuse_columns_and_ties_put_longer_first() {
        // A 0–60, B 30–90, C 60–120: C reuses A's column.
        assert_eq!(boxes(&[(0, 60), (30, 90), (60, 120)], 30), vec![cb(0, 1, 2), cb(1, 1, 2), cb(0, 1, 2)]);
        // Same start: the longer one goes first (left).
        assert_eq!(boxes(&[(0, 30), (0, 90)], 30), vec![cb(1, 1, 2), cb(0, 1, 2)]);
    }

    #[test]
    fn events_expand_into_free_columns() {
        // A 0–120 | B 0–30 | C 0–30, then D 60–90 lands in B's column and can
        // stretch over C's column, which is free from 30 on.
        let got = boxes(&[(0, 120), (0, 30), (0, 30), (60, 90)], 30);
        assert_eq!(got, vec![cb(0, 1, 3), cb(1, 1, 3), cb(2, 1, 3), cb(1, 2, 3)]);
    }

    #[test]
    fn short_events_get_a_minimum_visual_length() {
        // Two 5-minute events 10 minutes apart overlap once stretched to 30.
        assert_eq!(boxes(&[(0, 5), (10, 15)], 30), vec![cb(0, 1, 2), cb(1, 1, 2)]);
        assert_eq!(boxes(&[(0, 5), (10, 15)], 5), vec![cb(0, 1, 1), cb(0, 1, 1)]);
    }

    #[test]
    fn narrow_clusters_fold_into_a_more_chip() {
        // 207px fits seven 28px columns: up to 7 columns show as they are.
        assert_eq!(visible_columns(7, 207.0), None);
        // Nine don't: six events stay and the seventh column holds "+3".
        assert_eq!(visible_columns(9, 207.0), Some(6));
        // Always keep at least one event.
        assert_eq!(visible_columns(4, 40.0), Some(1));
        assert_eq!(visible_columns(3, 0.0), None);
    }

    #[test]
    fn lanes_pack_greedily() {
        // Mon–Wed, Tue, Thu–Fri, Wed–Thu (days 1..=5)
        let got = pack_lanes(&[(1, 3), (2, 2), (4, 5), (3, 4)]);
        assert_eq!(got, vec![0, 1, 0, 1]);
        assert_eq!(pack_lanes(&[(0, 6), (0, 0), (6, 6)]), vec![0, 1, 1]);
    }
}
