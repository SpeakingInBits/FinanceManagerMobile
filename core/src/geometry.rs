//! Chart geometry: donut arcs (like `d3.arc`) and a port of `d3-sankey`'s layout, emitted as SVG
//! path commands so the UI can draw them with plain path elements.

use std::f64::consts::{PI, TAU};
use std::fmt::Write;

use crate::charts::{CategoryGroupSlice, SankeyGraph};
use crate::model::Cents;

/// One drawable subcategory slice of the exploded donut.
#[derive(Clone, PartialEq, Debug)]
pub struct PieArc {
    pub path: String,
    pub color: String,
    /// "Category · Subcategory", or just the name when the slice is the whole category.
    pub heading: String,
    pub total: Cents,
    start_angle: f64,
    end_angle: f64,
    offset: (f64, f64),
}

/// Layout of the donut inside a `size` x `size` view box.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct PieLayout {
    pub size: f64,
    pub arcs: Vec<PieArc>,
    inner: f64,
    outer: f64,
}

impl PieLayout {
    /// The arc under a point in view-box coordinates, if any.
    pub fn hit_test(&self, x: f64, y: f64) -> Option<usize> {
        let c = self.size / 2.0;
        self.arcs.iter().position(|a| {
            let (dx, dy) = (x - c - a.offset.0, y - c - a.offset.1);
            let r = dx.hypot(dy);
            // Angle clockwise from 12 o'clock, in [0, 2π).
            let angle = dx.atan2(-dy).rem_euclid(TAU);
            r >= self.inner && r <= self.outer && angle >= a.start_angle && angle < a.end_angle
        })
    }
}

/// Lays out an "exploded" donut: each category is nudged outward along its bisector, with a gap
/// only between categories, and its subcategory slices kept contiguous.
pub fn pie_layout(groups: &[CategoryGroupSlice], size: f64) -> PieLayout {
    let total: Cents = groups.iter().map(|g| g.total).sum();
    if total <= 0 {
        return PieLayout {
            size,
            ..Default::default()
        };
    }
    let base_radius = size / 2.0 - 8.0;
    let multi = groups.len() > 1;
    let explode = if multi {
        (base_radius * 0.06).min(10.0)
    } else {
        0.0
    };
    let outer = base_radius - explode;
    let inner = outer * 0.55;
    let gap = if multi { 0.03 } else { 0.0 };
    let available = TAU - gap * groups.len() as f64;
    let c = size / 2.0;

    let mut arcs = Vec::new();
    let mut angle = 0.0;
    for group in groups {
        let span = group.total as f64 / total as f64 * available;
        let mid = angle + span / 2.0;
        let offset = (mid.sin() * explode, -mid.cos() * explode);
        let mut start = angle;
        for slice in &group.slices {
            let slice_span = if group.total > 0 {
                slice.total as f64 / group.total as f64 * span
            } else {
                0.0
            };
            let end = start + slice_span;
            if slice_span > 1e-9 {
                let heading = if group.category_name == slice.label {
                    slice.label.clone()
                } else {
                    format!("{} · {}", group.category_name, slice.label)
                };
                arcs.push(PieArc {
                    path: annular_sector(c + offset.0, c + offset.1, inner, outer, start, end),
                    color: slice.color.clone(),
                    heading,
                    total: slice.total,
                    start_angle: start,
                    end_angle: end,
                    offset,
                });
            }
            start = end;
        }
        angle += span + gap;
    }
    PieLayout {
        size,
        arcs,
        inner,
        outer,
    }
}

fn polar(cx: f64, cy: f64, r: f64, a: f64) -> (f64, f64) {
    (cx + r * a.sin(), cy - r * a.cos())
}

/// SVG path for a ring segment between `a0` and `a1` (radians clockwise from 12 o'clock).
fn annular_sector(cx: f64, cy: f64, r0: f64, r1: f64, a0: f64, a1: f64) -> String {
    let mut d = String::new();
    if a1 - a0 >= TAU - 1e-6 {
        // A full ring: two concentric circles, drawn in opposite directions to leave the hole.
        let ring = |d: &mut String, r: f64, sweep: u8| {
            let _ = write!(d, "M {:.2} {:.2} ", cx, cy - r);
            let _ = write!(d, "A {r:.2} {r:.2} 0 1 {sweep} {:.2} {:.2} ", cx, cy + r);
            let _ = write!(d, "A {r:.2} {r:.2} 0 1 {sweep} {:.2} {:.2} Z ", cx, cy - r);
        };
        ring(&mut d, r1, 1);
        ring(&mut d, r0, 0);
        return d;
    }
    let large = u8::from(a1 - a0 > PI);
    let (ox0, oy0) = polar(cx, cy, r1, a0);
    let (ox1, oy1) = polar(cx, cy, r1, a1);
    let (ix1, iy1) = polar(cx, cy, r0, a1);
    let (ix0, iy0) = polar(cx, cy, r0, a0);
    let _ = write!(
        d,
        "M {ox0:.2} {oy0:.2} A {r1:.2} {r1:.2} 0 {large} 1 {ox1:.2} {oy1:.2} \
         L {ix1:.2} {iy1:.2} A {r0:.2} {r0:.2} 0 {large} 0 {ix0:.2} {iy0:.2} Z"
    );
    d
}

/// A positioned Sankey node.
#[derive(Clone, PartialEq, Debug)]
pub struct SankeyNodeBox {
    pub x0: f64,
    pub x1: f64,
    pub y0: f64,
    pub y1: f64,
    pub value: Cents,
}

/// A positioned Sankey link: a filled band between two horizontal Béziers.
#[derive(Clone, PartialEq, Debug)]
pub struct SankeyLinkPath {
    pub source: usize,
    pub path: String,
    pub value: Cents,
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct SankeyLayout {
    pub nodes: Vec<SankeyNodeBox>,
    pub links: Vec<SankeyLinkPath>,
}

struct Node {
    value: f64,
    depth: usize,
    layer: usize,
    x0: f64,
    x1: f64,
    y0: f64,
    y1: f64,
    source_links: Vec<usize>,
    target_links: Vec<usize>,
}

struct Link {
    source: usize,
    target: usize,
    value: f64,
    width: f64,
    y0: f64,
    y1: f64,
}

/// Port of `d3-sankey` 0.12 with justify alignment, 6 relaxation iterations, and no custom
/// node/link sort, laid out within `[x0, y0]..[x1, y1]`.
pub struct Sankey {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    pub node_width: f64,
    pub node_padding: f64,
    pub iterations: usize,
}

impl Sankey {
    pub fn layout(&self, graph: &SankeyGraph) -> SankeyLayout {
        if graph.nodes.is_empty() || graph.links.is_empty() {
            return SankeyLayout::default();
        }
        let mut nodes: Vec<Node> = graph
            .nodes
            .iter()
            .map(|_| Node {
                value: 0.0,
                depth: 0,
                layer: 0,
                x0: 0.0,
                x1: 0.0,
                y0: 0.0,
                y1: 0.0,
                source_links: vec![],
                target_links: vec![],
            })
            .collect();
        let mut links: Vec<Link> = graph
            .links
            .iter()
            .map(|l| Link {
                source: l.source,
                target: l.target,
                value: l.value as f64,
                width: 0.0,
                y0: 0.0,
                y1: 0.0,
            })
            .collect();
        for (i, l) in links.iter().enumerate() {
            nodes[l.source].source_links.push(i);
            nodes[l.target].target_links.push(i);
        }
        // computeNodeValues
        for n in nodes.iter_mut() {
            let out: f64 = n.source_links.iter().map(|&l| links[l].value).sum();
            let inc: f64 = n.target_links.iter().map(|&l| links[l].value).sum();
            n.value = out.max(inc);
        }
        // computeNodeDepths (breadth-first from every node; the last visit wins).
        let n = nodes.len();
        let mut current: Vec<usize> = (0..n).collect();
        let mut x = 0;
        while !current.is_empty() {
            let mut next: Vec<usize> = Vec::new();
            for &i in &current {
                nodes[i].depth = x;
                for &l in &nodes[i].source_links {
                    let t = links[l].target;
                    if !next.contains(&t) {
                        next.push(t);
                    }
                }
            }
            x += 1;
            if x > n {
                // A cycle; the graph builder never produces one, but don't loop forever.
                return SankeyLayout::default();
            }
            current = next;
        }

        let columns = self.compute_node_layers(&mut nodes);
        let max_column = columns.iter().map(Vec::len).max().unwrap_or(1);
        let py = if max_column > 1 {
            self.node_padding
                .min((self.y1 - self.y0) / (max_column - 1) as f64)
        } else {
            self.node_padding
        };
        let mut columns = columns;
        self.initialize_node_breadths(&mut columns, &mut nodes, &mut links, py);
        for i in 0..self.iterations {
            let alpha = 0.99f64.powi(i as i32);
            let beta = (1.0 - alpha).max((i + 1) as f64 / self.iterations as f64);
            self.relax_right_to_left(&mut columns, &mut nodes, &links, alpha, beta, py);
            self.relax_left_to_right(&mut columns, &mut nodes, &links, alpha, beta, py);
        }
        // computeLinkBreadths
        for node in &nodes {
            let mut y0 = node.y0;
            let mut y1 = node.y0;
            for &l in &node.source_links {
                links[l].y0 = y0 + links[l].width / 2.0;
                y0 += links[l].width;
            }
            for &l in &node.target_links {
                links[l].y1 = y1 + links[l].width / 2.0;
                y1 += links[l].width;
            }
        }

        SankeyLayout {
            nodes: nodes
                .iter()
                .map(|n| SankeyNodeBox { x0: n.x0, x1: n.x1, y0: n.y0, y1: n.y1, value: n.value.round() as Cents })
                .collect(),
            links: links
                .iter()
                .map(|l| {
                    let (sx, tx) = (nodes[l.source].x1, nodes[l.target].x0);
                    let mx = (sx + tx) / 2.0;
                    // The band d3's `sankeyLinkHorizontal` strokes, as a filled outline: renderers
                    // cope better with a fill than with very wide strokes.
                    let half = l.width.max(1.0) / 2.0;
                    let (a0, a1, b0, b1) = (l.y0 - half, l.y1 - half, l.y0 + half, l.y1 + half);
                    SankeyLinkPath {
                        source: l.source,
                        path: format!(
                            "M {sx:.2} {a0:.2} C {mx:.2} {a0:.2} {mx:.2} {a1:.2} {tx:.2} {a1:.2}                              L {tx:.2} {b1:.2} C {mx:.2} {b1:.2} {mx:.2} {b0:.2} {sx:.2} {b0:.2} Z"
                        ),
                        value: l.value.round() as Cents,
                    }
                })
                .collect(),
        }
    }

    fn compute_node_layers(&self, nodes: &mut [Node]) -> Vec<Vec<usize>> {
        let x = nodes.iter().map(|n| n.depth).max().unwrap_or(0) + 1;
        let kx = if x > 1 {
            (self.x1 - self.x0 - self.node_width) / (x - 1) as f64
        } else {
            0.0
        };
        let mut columns = vec![Vec::new(); x];
        for (i, node) in nodes.iter_mut().enumerate() {
            // sankeyJustify: nodes without outgoing links go to the last column.
            let layer = if node.source_links.is_empty() {
                x - 1
            } else {
                node.depth.min(x - 1)
            };
            node.layer = layer;
            node.x0 = self.x0 + layer as f64 * kx;
            node.x1 = node.x0 + self.node_width;
            columns[layer].push(i);
        }
        columns.retain(|c| !c.is_empty());
        columns
    }

    fn initialize_node_breadths(
        &self,
        columns: &mut [Vec<usize>],
        nodes: &mut [Node],
        links: &mut [Link],
        py: f64,
    ) {
        let ky = columns
            .iter()
            .map(|c| {
                let sum: f64 = c.iter().map(|&i| nodes[i].value).sum();
                if sum > 0.0 {
                    (self.y1 - self.y0 - (c.len() - 1) as f64 * py) / sum
                } else {
                    f64::INFINITY
                }
            })
            .fold(f64::INFINITY, f64::min);
        let ky = if ky.is_finite() { ky } else { 0.0 };
        for column in columns.iter() {
            let mut y = self.y0;
            for &i in column {
                nodes[i].y0 = y;
                nodes[i].y1 = y + nodes[i].value * ky;
                y = nodes[i].y1 + py;
                for &l in &nodes[i].source_links {
                    links[l].width = links[l].value * ky;
                }
            }
            let spread = (self.y1 - y + py) / (column.len() + 1) as f64;
            for (k, &i) in column.iter().enumerate() {
                nodes[i].y0 += spread * (k + 1) as f64;
                nodes[i].y1 += spread * (k + 1) as f64;
            }
            reorder_links(column, nodes, links);
        }
    }

    fn relax_left_to_right(
        &self,
        columns: &mut [Vec<usize>],
        nodes: &mut [Node],
        links: &[Link],
        alpha: f64,
        beta: f64,
        py: f64,
    ) {
        for column in columns.iter_mut().skip(1) {
            for &target in column.iter() {
                let (mut y, mut w) = (0.0, 0.0);
                for &l in &nodes[target].target_links {
                    let source = links[l].source;
                    let v =
                        links[l].value * (nodes[target].layer as f64 - nodes[source].layer as f64);
                    y += target_top(nodes, links, source, target, py) * v;
                    w += v;
                }
                if w <= 0.0 || w.is_nan() {
                    continue;
                }
                let dy = (y / w - nodes[target].y0) * alpha;
                nodes[target].y0 += dy;
                nodes[target].y1 += dy;
                reorder_node_links(target, nodes, links);
            }
            column.sort_by(|&a, &b| nodes[a].y0.total_cmp(&nodes[b].y0));
            self.resolve_collisions(column, nodes, beta, py);
        }
    }

    fn relax_right_to_left(
        &self,
        columns: &mut [Vec<usize>],
        nodes: &mut [Node],
        links: &[Link],
        alpha: f64,
        beta: f64,
        py: f64,
    ) {
        for c in (0..columns.len().saturating_sub(1)).rev() {
            for &source in &columns[c] {
                let (mut y, mut w) = (0.0, 0.0);
                for &l in &nodes[source].source_links {
                    let target = links[l].target;
                    let v =
                        links[l].value * (nodes[target].layer as f64 - nodes[source].layer as f64);
                    y += source_top(nodes, links, source, target, py) * v;
                    w += v;
                }
                if w <= 0.0 || w.is_nan() {
                    continue;
                }
                let dy = (y / w - nodes[source].y0) * alpha;
                nodes[source].y0 += dy;
                nodes[source].y1 += dy;
                reorder_node_links(source, nodes, links);
            }
            columns[c].sort_by(|&a, &b| nodes[a].y0.total_cmp(&nodes[b].y0));
            self.resolve_collisions(&columns[c], nodes, beta, py);
        }
    }

    fn resolve_collisions(&self, column: &[usize], nodes: &mut [Node], alpha: f64, py: f64) {
        let i = column.len() >> 1;
        let subject = column[i];
        let (sy0, sy1) = (nodes[subject].y0, nodes[subject].y1);
        bottom_to_top(column, nodes, sy0 - py, i as isize - 1, alpha, py);
        top_to_bottom(column, nodes, sy1 + py, i + 1, alpha, py);
        bottom_to_top(column, nodes, self.y1, column.len() as isize - 1, alpha, py);
        top_to_bottom(column, nodes, self.y0, 0, alpha, py);
    }
}

/// Push any overlapping nodes down.
fn top_to_bottom(
    column: &[usize],
    nodes: &mut [Node],
    mut y: f64,
    from: usize,
    alpha: f64,
    py: f64,
) {
    for &i in &column[from.min(column.len())..] {
        let dy = (y - nodes[i].y0) * alpha;
        if dy > 1e-6 {
            nodes[i].y0 += dy;
            nodes[i].y1 += dy;
        }
        y = nodes[i].y1 + py;
    }
}

/// Push any overlapping nodes up.
fn bottom_to_top(
    column: &[usize],
    nodes: &mut [Node],
    mut y: f64,
    from: isize,
    alpha: f64,
    py: f64,
) {
    let mut k = from;
    while k >= 0 {
        let i = column[k as usize];
        let dy = (nodes[i].y1 - y) * alpha;
        if dy > 1e-6 {
            nodes[i].y0 -= dy;
            nodes[i].y1 -= dy;
        }
        y = nodes[i].y0 - py;
        k -= 1;
    }
}

fn sort_by_target_breadth(list: &mut [usize], nodes: &[Node], links: &[Link]) {
    list.sort_by(|&a, &b| {
        nodes[links[a].target]
            .y0
            .total_cmp(&nodes[links[b].target].y0)
            .then(a.cmp(&b))
    });
}

fn sort_by_source_breadth(list: &mut [usize], nodes: &[Node], links: &[Link]) {
    list.sort_by(|&a, &b| {
        nodes[links[a].source]
            .y0
            .total_cmp(&nodes[links[b].source].y0)
            .then(a.cmp(&b))
    });
}

fn reorder_links(column: &[usize], nodes: &mut [Node], links: &[Link]) {
    for &i in column {
        let mut s = std::mem::take(&mut nodes[i].source_links);
        sort_by_target_breadth(&mut s, nodes, links);
        nodes[i].source_links = s;
        let mut t = std::mem::take(&mut nodes[i].target_links);
        sort_by_source_breadth(&mut t, nodes, links);
        nodes[i].target_links = t;
    }
}

fn reorder_node_links(node: usize, nodes: &mut [Node], links: &[Link]) {
    for l in nodes[node].target_links.clone() {
        let src = links[l].source;
        let mut s = std::mem::take(&mut nodes[src].source_links);
        sort_by_target_breadth(&mut s, nodes, links);
        nodes[src].source_links = s;
    }
    for l in nodes[node].source_links.clone() {
        let tgt = links[l].target;
        let mut t = std::mem::take(&mut nodes[tgt].target_links);
        sort_by_source_breadth(&mut t, nodes, links);
        nodes[tgt].target_links = t;
    }
}

/// The target.y0 that would produce an ideal link from source to target.
fn target_top(nodes: &[Node], links: &[Link], source: usize, target: usize, py: f64) -> f64 {
    let s = &nodes[source];
    let mut y = s.y0 - (s.source_links.len() as f64 - 1.0) * py / 2.0;
    for &l in &s.source_links {
        if links[l].target == target {
            break;
        }
        y += links[l].width + py;
    }
    for &l in &nodes[target].target_links {
        if links[l].source == source {
            break;
        }
        y -= links[l].width;
    }
    y
}

/// The source.y0 that would produce an ideal link from source to target.
fn source_top(nodes: &[Node], links: &[Link], source: usize, target: usize, py: f64) -> f64 {
    let t = &nodes[target];
    let mut y = t.y0 - (t.target_links.len() as f64 - 1.0) * py / 2.0;
    for &l in &t.target_links {
        if links[l].source == source {
            break;
        }
        y += links[l].width + py;
    }
    for &l in &nodes[source].source_links {
        if links[l].target == target {
            break;
        }
        y -= links[l].width;
    }
    y
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::charts::{CategorySubSlice, SankeyLinkDatum, SankeyNodeDatum};

    fn group(name: &str, totals: &[Cents]) -> CategoryGroupSlice {
        CategoryGroupSlice {
            category_id: name.into(),
            category_name: name.into(),
            color: "#000".into(),
            total: totals.iter().sum(),
            slices: totals
                .iter()
                .enumerate()
                .map(|(i, t)| CategorySubSlice {
                    category_id: name.into(),
                    subcategory_id: (totals.len() > 1).then(|| format!("{name}{i}")),
                    label: if totals.len() > 1 {
                        format!("{name}{i}")
                    } else {
                        name.into()
                    },
                    total: *t,
                    color: "#000".into(),
                })
                .collect(),
        }
    }

    #[test]
    fn pie_hit_test_finds_slices() {
        let layout = pie_layout(&[group("A", &[300, 100]), group("B", &[100])], 300.0);
        assert_eq!(layout.arcs.len(), 3);
        assert_eq!(layout.arcs[0].heading, "A · A0");
        assert_eq!(layout.arcs[2].heading, "B");
        // Mid-ring, a little clockwise of 12 o'clock: first slice of the first category.
        let ring_mid = (layout.inner + layout.outer) / 2.0;
        let (ox, oy) = layout.arcs[0].offset;
        let (x, y) = polar(150.0 + ox, 150.0 + oy, ring_mid, 0.3);
        assert_eq!(layout.hit_test(x, y), Some(0));
        let (x, y) = polar(150.0 + ox, 150.0 + oy, ring_mid, 4.2);
        assert_eq!(layout.hit_test(x, y), Some(1));
        // The centre hole hits nothing.
        assert_eq!(layout.hit_test(150.0, 150.0), None);
    }

    #[test]
    fn single_full_ring_is_two_circles() {
        let layout = pie_layout(&[group("A", &[100])], 200.0);
        assert_eq!(layout.arcs.len(), 1);
        assert_eq!(layout.arcs[0].path.matches('M').count(), 2);
    }

    #[test]
    fn sankey_layout_conserves_flow_and_stays_in_bounds() {
        let node = |name: &str| SankeyNodeDatum {
            name: name.into(),
            color: "#000".into(),
            is_budget: false,
        };
        let graph = SankeyGraph {
            nodes: vec![
                node("Pay"),
                node("Gift"),
                node("Total"),
                node("Rent"),
                node("Food"),
            ],
            links: vec![
                SankeyLinkDatum {
                    source: 0,
                    target: 2,
                    value: 800,
                },
                SankeyLinkDatum {
                    source: 1,
                    target: 2,
                    value: 200,
                },
                SankeyLinkDatum {
                    source: 2,
                    target: 3,
                    value: 600,
                },
                SankeyLinkDatum {
                    source: 2,
                    target: 4,
                    value: 300,
                },
            ],
        };
        let s = Sankey {
            x0: 4.0,
            y0: 4.0,
            x1: 396.0,
            y1: 316.0,
            node_width: 14.0,
            node_padding: 12.0,
            iterations: 6,
        };
        let l = s.layout(&graph);
        assert_eq!(l.nodes.len(), 5);
        assert_eq!(l.nodes[2].value, 1000);
        // Columns: sources at the left edge, sinks at the right edge.
        assert_eq!(l.nodes[0].x0, 4.0);
        assert_eq!(l.nodes[3].x1, 396.0);
        for n in &l.nodes {
            assert!(n.y0 >= 4.0 - 1e-6 && n.y1 <= 316.0 + 1e-6, "{n:?}");
        }
        let outgoing: Cents = l
            .links
            .iter()
            .filter(|k| k.source == 2)
            .map(|k| k.value)
            .sum();
        assert_eq!(outgoing, 900);
        assert!(l.links[0].path.ends_with('Z'));
    }
}
