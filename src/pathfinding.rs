use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use crate::world::{is_base_cell, is_obstacle};

pub(crate) fn bfs(
    map: &Arc<Vec<Vec<f64>>>,
    _collected_crystals: &HashSet<(u16, u16)>,
    _collected_energy: &HashSet<(u16, u16)>,
    from: (u16, u16),
    to: (u16, u16),
    width: usize,
    height: usize,
    base_pos: (u16, u16),
) -> VecDeque<(u16, u16)> {
    let mut visited: HashSet<(u16, u16)> = HashSet::new();
    let mut queue: VecDeque<(u16, u16, Vec<(u16, u16)>)> = VecDeque::new();

    queue.push_back((from.0, from.1, vec![]));
    visited.insert(from);

    while let Some((x, y, path)) = queue.pop_front() {
        if (x, y) == to {
            return path.into();
        }

        for (dx, dy) in [(0i16, -1), (0, 1), (-1, 0), (1, 0)] {
            let nx = (x as i16 + dx).clamp(1, width as i16 - 2) as u16;
            let ny = (y as i16 + dy).clamp(1, height as i16 - 2) as u16;

            if visited.contains(&(nx, ny)) {
                continue;
            }

            let passable = !is_obstacle(map, nx, ny)
                && (!is_base_cell(nx, ny, base_pos) || is_base_cell(to.0, to.1, base_pos));

            if passable {
                visited.insert((nx, ny));
                let mut new_path = path.clone();
                new_path.push((nx, ny));
                queue.push_back((nx, ny, new_path));
            }
        }
    }

    VecDeque::new()
}
