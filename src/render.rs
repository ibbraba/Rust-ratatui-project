use crate::app::{RobotType, SimulationState};
use crate::world::is_base_cell;
use ratatui::prelude::{Rect, Widget};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::Line;

pub(crate) fn render_world(state: &SimulationState, area: Rect, buf: &mut ratatui::prelude::Buffer) {
    for y in 0..area.height as usize {
        for x in 0..area.width as usize {
            if is_base_cell(x as u16, y as u16, state.base_pos) {
                continue;
            }

            if x == 0 || x == area.width as usize - 1 || y == 0 || y == area.height as usize - 1 {
                continue;
            }

            if y >= state.height || x >= state.width {
                continue;
            }

            let value = state.map[y][x];

            let (symbol, color) = if state.collected_crystals.contains(&(x as u16, y as u16)) {
                (" ", Color::DarkGray)
            } else if state.collected_energy.contains(&(x as u16, y as u16)) {
                (" ", Color::DarkGray)
            } else if state.discovered_crystals.contains(&(x as u16, y as u16)) {
                ("C", Color::LightYellow)
            } else if state.discovered_energy.contains(&(x as u16, y as u16)) {
                ("E", Color::LightGreen)
            } else {
                render_cell(value)
            };

            buf[(area.x + x as u16, area.y + y as u16)]
                .set_symbol(symbol)
                .set_style(Style::default().fg(color));
        }
    }

    render_base(state.base_pos, area, buf);

    Line::from("Robots Game — Appuyez sur n'importe quelle touche pour quitter")
        .bold()
        .yellow()
        .render(area, buf);

    let summary = format!(
        " Cristaux collectés: {}  |  Énergie collectée: {}  |  Cristaux découverts: {}  |  Énergie découverte: {} ",
        state.deposited_crystals,
        state.deposited_energy,
        state.discovered_crystals.len(),
        state.discovered_energy.len(),
    );

    let bottom_area = Rect {
        x: area.x,
        y: area.y + area.height - 1,
        width: area.width,
        height: 1,
    };

    Line::from(summary)
        .bold()
        .light_blue()
        .centered()
        .render(bottom_area, buf);

    for robot in &state.robots {
        let (rx, ry) = robot.position;
        let (symbol, color) = match robot.robot_type {
            RobotType::Scout => ("x", Color::LightRed),
            RobotType::Collector => ("o", Color::Magenta),
        };

        buf[(area.x + rx, area.y + ry)]
            .set_symbol(symbol)
            .set_style(Style::default().fg(color).bold());
    }
}

fn render_cell(value: f64) -> (&'static str, Color) {
    match value {
        v if v < -0.1 => ("0", Color::Cyan),
        v if v < 0.15 => (" ", Color::DarkGray),
        v if v < 0.30 => ("C", Color::LightMagenta),
        v if v < 0.45 => (" ", Color::DarkGray),
        v if v < 0.60 => ("E", Color::Green),
        _ => (" ", Color::White),
    }
}

fn render_base(pos: (u16, u16), area: Rect, buf: &mut ratatui::prelude::Buffer) {
    let tiles: &[(&str, &str, &str)] = &[("#", "#", "#"), ("#", "#", "#"), ("#", "#", "#")];

    let base_style = Style::default().fg(Color::Green).bold();

    for (row, (left, center, right)) in tiles.iter().enumerate() {
        let y = area.y + pos.1 + row as u16 - 1;
        let x = area.x + pos.0;

        if y < area.y || y >= area.y + area.height {
            continue;
        }

        for (col, symbol) in [left, center, right].iter().enumerate() {
            let cx = x + col as u16 - 1;
            if cx < area.x || cx >= area.x + area.width {
                continue;
            }
            buf[(cx, y)].set_symbol(symbol).set_style(base_style);
        }
    }
}
