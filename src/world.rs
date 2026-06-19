pub(crate) fn map_value(map: &[Vec<f64>], x: u16, y: u16) -> f64 {
    map[y as usize % map.len()][x as usize % map[0].len()]
}

pub(crate) fn is_obstacle(map: &[Vec<f64>], x: u16, y: u16) -> bool {
    map_value(map, x, y) < -0.1
}

pub(crate) fn is_crystal(map: &[Vec<f64>], x: u16, y: u16) -> bool {
    let v = map_value(map, x, y);
    v >= 0.15 && v < 0.30
}

pub(crate) fn is_energy(map: &[Vec<f64>], x: u16, y: u16) -> bool {
    let v = map_value(map, x, y);
    v >= 0.45 && v < 0.60
}

pub(crate) fn is_base_cell(x: u16, y: u16, pos: (u16, u16)) -> bool {
    let (bx, by) = pos;
    let x_min = bx.saturating_sub(1);
    let y_min = by.saturating_sub(1);
    let x_max = bx + 1;
    let y_max = by + 1;

    x >= x_min && x <= x_max && y >= y_min && y <= y_max
}
