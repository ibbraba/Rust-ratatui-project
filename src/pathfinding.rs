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

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_map(obstacles: &[(usize, usize)]) -> Arc<Vec<Vec<f64>>> {
        create_test_map_sized(5, 5, obstacles)
    }

    fn create_test_map_sized(width: usize, height: usize, obstacles: &[(usize, usize)]) -> Arc<Vec<Vec<f64>>> {
        let mut map = vec![vec![0.0; width]; height];
        for &(x, y) in obstacles {
            map[y][x] = -0.2; // obstacle
        }
        Arc::new(map)
    }

    #[test]
    fn test_bfs_shortest_path_empty_grid() {
        let map = create_test_map(&[]);
        let collected_crystals = HashSet::new();
        let collected_energy = HashSet::new();
        let base_pos = (4, 4);

        let path = bfs(
            &map,
            &collected_crystals,
            &collected_energy,
            (1, 1),
            (3, 1),
            5,
            5,
            base_pos,
        );

        let expected: VecDeque<(u16, u16)> = vec![(2, 1), (3, 1)].into();
        assert_eq!(path, expected);
    }

    #[test]
    fn test_bfs_routes_around_obstacle() {
        let map = create_test_map(&[(2, 1)]);
        let collected_crystals = HashSet::new();
        let collected_energy = HashSet::new();
        let base_pos = (4, 4);

        let path = bfs(
            &map,
            &collected_crystals,
            &collected_energy,
            (1, 1),
            (3, 1),
            5,
            5,
            base_pos,
        );

        assert!(!path.contains(&(2, 1)));
        assert!(!path.is_empty());
        assert_eq!(path.back(), Some(&(3, 1)));
    }

    #[test]
    fn test_bfs_respects_base_boundaries() {
        let map = create_test_map_sized(20, 20, &[]);
        let collected_crystals = HashSet::new();
        let collected_energy = HashSet::new();
        let base_pos = (10, 10);

        let path = bfs(
            &map,
            &collected_crystals,
            &collected_energy,
            (2, 10),
            (18, 10),
            20,
            20,
            base_pos,
        );

        for &pos in &path {
            assert!(!is_base_cell(pos.0, pos.1, base_pos), "Path must not cross base cells: {:?}", pos);
        }
        assert!(!path.is_empty());
    }

    #[test]
    fn test_bfs_allows_entry_to_base_if_target() {
        let map = create_test_map_sized(20, 20, &[]);
        let collected_crystals = HashSet::new();
        let collected_energy = HashSet::new();
        let base_pos = (10, 10);

        let path = bfs(
            &map,
            &collected_crystals,
            &collected_energy,
            (2, 10),
            (10, 10),
            20,
            20,
            base_pos,
        );

        assert!(!path.is_empty());
        assert_eq!(path.back(), Some(&(10, 10)));
    }

    #[test]
    fn test_bfs_no_path() {
        let map = create_test_map_sized(10, 10, &[(4, 5), (6, 5), (5, 4), (5, 6)]);
        let collected_crystals = HashSet::new();
        let collected_energy = HashSet::new();
        let base_pos = (0, 0);

        let path = bfs(
            &map,
            &collected_crystals,
            &collected_energy,
            (1, 1),
            (5, 5),
            10,
            10,
            base_pos,
        );

        assert!(path.is_empty(), "Expected empty path, got: {:?}", path);
    }
}
