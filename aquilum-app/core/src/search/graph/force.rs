const THETA: f64 = 0.9;
const COOLING: f64 = 0.94;
const SETTLED_STEP: f64 = 2e-3;
const STIFFNESS: f64 = 0.9;
const NO_CELL: u32 = u32::MAX;
const MAX_DEPTH: u32 = 24;
const GOLDEN_ANGLE: f64 = 2.399_963_229_728_653;

pub struct Settings {
    pub ticks: usize,
    pub spacing: f64,
    pub repulsion: f64,
}

pub fn refine(coordinates: &mut [f64], edges: &[(u32, u32)], settings: &Settings) {
    let count = coordinates.len() / 2;
    if count < 3 || settings.ticks == 0 {
        return;
    }
    let spacing = settings.spacing;
    let strength = settings.repulsion * spacing * spacing;
    separate_coincident(coordinates, spacing);
    let mut displacement = vec![0.0f64; coordinates.len()];
    let mut stack = Vec::with_capacity(64);
    // Retain Quadtree cell storage across physics ticks. The tree's cell
    // contents, insert order and all force arithmetic are rebuilt unchanged.
    let mut tree = Quadtree { cells: Vec::with_capacity(count * 2) };
    let mut temperature = spacing * 2.0;
    for _ in 0..settings.ticks {
        displacement.fill(0.0);
        tree.rebuild(coordinates);
        for node in 0..count {
            tree.repel(node, coordinates, spacing, strength, &mut displacement, &mut stack);
        }
        for (left, right) in edges {
            attract(
                *left as usize,
                *right as usize,
                coordinates,
                spacing,
                &mut displacement,
            );
        }
        let mut furthest = 0.0f64;
        for node in 0..count {
            let (x, y) = (displacement[node * 2], displacement[node * 2 + 1]);
            let length = x.hypot(y);
            if length < f64::EPSILON {
                continue;
            }
            let travelled = length.min(temperature);
            let step = travelled / length;
            coordinates[node * 2] += x * step;
            coordinates[node * 2 + 1] += y * step;
            furthest = furthest.max(travelled);
        }
        if furthest < spacing * SETTLED_STEP {
            return;
        }
        temperature *= COOLING;
    }
}

fn separate_coincident(coordinates: &mut [f64], spacing: f64) {
    let radius = spacing * 1e-3;
    for (node, pair) in coordinates.chunks_exact_mut(2).enumerate() {
        let angle = node as f64 * GOLDEN_ANGLE;
        pair[0] += radius * angle.cos();
        pair[1] += radius * angle.sin();
    }
}

fn attract(
    left: usize,
    right: usize,
    coordinates: &[f64],
    spacing: f64,
    displacement: &mut [f64],
) {
    let dx = coordinates[left * 2] - coordinates[right * 2];
    let dy = coordinates[left * 2 + 1] - coordinates[right * 2 + 1];
    let distance = dx.hypot(dy).max(1e-9);
    let force = STIFFNESS * (distance - spacing);
    let (ux, uy) = (dx / distance, dy / distance);
    displacement[left * 2] -= ux * force;
    displacement[left * 2 + 1] -= uy * force;
    displacement[right * 2] += ux * force;
    displacement[right * 2 + 1] += uy * force;
}

struct Cell {
    mass: f64,
    sum_x: f64,
    sum_y: f64,
    origin_x: f64,
    origin_y: f64,
    size: f64,
    children: [u32; 4],
    point: u32,
}

impl Cell {
    fn has_children(&self) -> bool {
        self.children.iter().any(|child| *child != NO_CELL)
    }
}

struct Quadtree {
    cells: Vec<Cell>,
}

impl Quadtree {
    fn rebuild(&mut self, coordinates: &[f64]) {
        let count = coordinates.len() / 2;
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for pair in coordinates.chunks_exact(2) {
            min_x = min_x.min(pair[0]);
            min_y = min_y.min(pair[1]);
            max_x = max_x.max(pair[0]);
            max_y = max_y.max(pair[1]);
        }
        let size = (max_x - min_x).max(max_y - min_y).max(1e-6) * 1.01;
        self.cells.clear();
        self.push_cell(min_x, min_y, size);
        for node in 0..count {
            self.insert(0, node as u32, coordinates, 0);
        }
    }

    fn push_cell(&mut self, origin_x: f64, origin_y: f64, size: f64) -> u32 {
        self.cells.push(Cell {
            mass: 0.0,
            sum_x: 0.0,
            sum_y: 0.0,
            origin_x,
            origin_y,
            size,
            children: [NO_CELL; 4],
            point: NO_CELL,
        });
        self.cells.len() as u32 - 1
    }

    fn insert(&mut self, cell: u32, node: u32, coordinates: &[f64], depth: u32) {
        let index = cell as usize;
        self.cells[index].mass += 1.0;
        self.cells[index].sum_x += coordinates[node as usize * 2];
        self.cells[index].sum_y += coordinates[node as usize * 2 + 1];
        if self.cells[index].mass == 1.0 {
            self.cells[index].point = node;
            return;
        }
        if depth >= MAX_DEPTH {
            return;
        }
        if let Some(resident) = self.take_point(index) {
            let quadrant = self.quadrant(index, coordinates, resident);
            let child = self.ensure_child(index, quadrant);
            self.insert(child, resident, coordinates, depth + 1);
        }
        let quadrant = self.quadrant(index, coordinates, node);
        let child = self.ensure_child(index, quadrant);
        self.insert(child, node, coordinates, depth + 1);
    }

    fn take_point(&mut self, index: usize) -> Option<u32> {
        let resident = self.cells[index].point;
        if resident == NO_CELL {
            return None;
        }
        self.cells[index].point = NO_CELL;
        Some(resident)
    }

    fn quadrant(&self, index: usize, coordinates: &[f64], node: u32) -> usize {
        let cell = &self.cells[index];
        let middle = cell.size / 2.0;
        let right = coordinates[node as usize * 2] >= cell.origin_x + middle;
        let top = coordinates[node as usize * 2 + 1] >= cell.origin_y + middle;
        usize::from(right) + usize::from(top) * 2
    }

    fn ensure_child(&mut self, index: usize, quadrant: usize) -> u32 {
        let existing = self.cells[index].children[quadrant];
        if existing != NO_CELL {
            return existing;
        }
        let half = self.cells[index].size / 2.0;
        let origin_x = self.cells[index].origin_x + if quadrant % 2 == 1 { half } else { 0.0 };
        let origin_y = self.cells[index].origin_y + if quadrant >= 2 { half } else { 0.0 };
        let child = self.push_cell(origin_x, origin_y, half);
        self.cells[index].children[quadrant] = child;
        child
    }

    fn repel(
        &self,
        node: usize,
        coordinates: &[f64],
        spacing: f64,
        strength: f64,
        displacement: &mut [f64],
        stack: &mut Vec<u32>,
    ) {
        let x = coordinates[node * 2];
        let y = coordinates[node * 2 + 1];
        stack.clear();
        stack.push(0);
        let mut push_x = 0.0;
        let mut push_y = 0.0;
        let floor = spacing * 0.05;
        while let Some(index) = stack.pop() {
            let cell = &self.cells[index as usize];
            if cell.mass == 0.0 || (cell.point != NO_CELL && cell.point as usize == node) {
                continue;
            }
            let dx = x - cell.sum_x / cell.mass;
            let dy = y - cell.sum_y / cell.mass;
            let distance = dx.hypot(dy);
            if !cell.has_children() || cell.size / distance.max(1e-12) < THETA {
                let force = strength * cell.mass / distance.max(floor);
                if distance < floor {
                    let angle = node as f64 * GOLDEN_ANGLE;
                    push_x += angle.cos() * force;
                    push_y += angle.sin() * force;
                } else {
                    push_x += dx / distance * force;
                    push_y += dy / distance * force;
                }
                continue;
            }
            for child in cell.children {
                if child != NO_CELL {
                    stack.push(child);
                }
            }
        }
        displacement[node * 2] += push_x;
        displacement[node * 2 + 1] += push_y;
    }
}

#[cfg(test)]
mod tests {
    use super::{refine, Quadtree, Settings};

    #[test]
    fn rebuilding_quadtree_preserves_cells_and_retained_capacity() {
        let positions = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 2.0, 2.0, -1.0, 0.0];
        let mut tree = Quadtree { cells: Vec::new() };
        tree.rebuild(&positions);
        let capacity = tree.cells.capacity();
        let snapshot = tree.cells.iter().map(|cell| (
            cell.mass.to_bits(), cell.sum_x.to_bits(), cell.sum_y.to_bits(),
            cell.origin_x.to_bits(), cell.origin_y.to_bits(), cell.size.to_bits(),
            cell.children, cell.point,
        )).collect::<Vec<_>>();
        for _ in 0..16 {
            tree.rebuild(&positions);
            let actual = tree.cells.iter().map(|cell| (
                cell.mass.to_bits(), cell.sum_x.to_bits(), cell.sum_y.to_bits(),
                cell.origin_x.to_bits(), cell.origin_y.to_bits(), cell.size.to_bits(),
                cell.children, cell.point,
            )).collect::<Vec<_>>();
            assert_eq!(actual, snapshot);
            assert_eq!(tree.cells.capacity(), capacity);
            assert_eq!(tree.cells[0].mass, (positions.len() / 2) as f64);
        }
    }

    fn settings(ticks: usize) -> Settings {
        Settings {
            ticks,
            spacing: 1.0,
            repulsion: 1.0,
        }
    }

    fn distance(coordinates: &[f64], left: usize, right: usize) -> f64 {
        let dx = coordinates[left * 2] - coordinates[right * 2];
        let dy = coordinates[left * 2 + 1] - coordinates[right * 2 + 1];
        dx.hypot(dy)
    }

    #[test]
    fn a_settled_layout_stops_paying_for_more_ticks() {
        let start = vec![0.0, 0.0, 1.0, 0.0, 0.5, 0.9, -0.5, 0.9, -1.0, 0.0];
        let edges = [(0u32, 1u32), (1, 2), (2, 3), (3, 4), (4, 0)];
        let mut budget = start.clone();
        let mut lavish = start;

        refine(&mut budget, &edges, &settings(400));
        refine(&mut lavish, &edges, &settings(4_000));

        assert_eq!(budget, lavish);
    }

    #[test]
    fn crowded_nodes_are_pushed_apart() {
        let mut coordinates = vec![0.0, 0.0, 0.02, 0.0, -0.02, 0.01, 0.0, 0.03];

        refine(&mut coordinates, &[], &settings(80));

        for left in 0..4 {
            for right in (left + 1)..4 {
                assert!(
                    distance(&coordinates, left, right) > 0.3,
                    "nodes {left} and {right} stayed on top of each other"
                );
            }
        }
    }

    #[test]
    fn linked_nodes_stay_closer_than_unlinked_ones() {
        let mut coordinates = vec![0.0, 0.0, 3.0, 0.0, -3.0, 0.5, 0.5, 3.0];

        refine(&mut coordinates, &[(0, 1)], &settings(120));

        assert!(distance(&coordinates, 0, 1) < distance(&coordinates, 0, 2));
        assert!(distance(&coordinates, 0, 1) < distance(&coordinates, 0, 3));
    }

    #[test]
    fn refinement_is_reproducible() {
        let start = vec![0.0, 0.0, 1.0, 0.2, -1.0, 0.4, 0.3, 1.0, -0.7, -0.9];
        let mut first = start.clone();
        let mut second = start;

        refine(&mut first, &[(0, 1), (1, 2), (2, 3)], &settings(40));
        refine(&mut second, &[(0, 1), (1, 2), (2, 3)], &settings(40));

        assert_eq!(first, second);
    }

    #[test]
    fn identical_positions_are_separated_instead_of_hanging() {
        let mut coordinates = vec![0.0; 12];

        refine(&mut coordinates, &[(0, 1), (1, 2), (0, 2)], &settings(80));

        assert!(coordinates.iter().all(|value| value.is_finite()));
        assert!(distance(&coordinates, 3, 4) > 0.2);
    }

    #[test]
    fn too_few_nodes_are_left_untouched() {
        let mut coordinates = vec![1.0, 2.0, 3.0, 4.0];

        refine(&mut coordinates, &[(0, 1)], &settings(50));

        assert_eq!(coordinates, vec![1.0, 2.0, 3.0, 4.0]);
    }
}
