use std::io::{self, Stdout};
use crossterm::event::KeyEventKind;
use noise::{NoiseFn, Perlin};
use ratatui::backend::CrosstermBackend;
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::Line;
use ratatui::widgets::Widget;
use ratatui::Terminal;
use ratatui::prelude::Rect;


fn main() -> io::Result<()> {
    let mut terminal: Terminal<CrosstermBackend<Stdout>> = ratatui::init();

    let size = terminal.size()?;
    let mut app = App::new(size.width as usize, size.height as usize);

    let app_result = app.run(&mut terminal);

    ratatui::restore();
    app_result
}

pub struct App {
    exit: bool,
    map: Vec<Vec<f64>>,
    width: usize,
    height: usize,
    base_pos: (u16, u16), 
    robots: Vec<Robot>,
}

pub struct Robot {
    pub position: (u16, u16),
}

impl Robot {
    pub fn new(position: (u16, u16)) -> Self {
        Self { position }
    }
    
}

impl App {
    pub fn new(width: usize, height: usize) -> Self {
        let base_pos = (width as u16 / 2, height as u16 / 2);
        Self {
            exit: false,
            map: Self::generate_map(width, height),
            width,
            height,
            base_pos,
            robots: vec![Robot::new((base_pos.0 + 5, base_pos.1))], // Example robot starting near the base
        }
    }

    fn generate_map(width: usize, height: usize) -> Vec<Vec<f64>> {
        let perlin = Perlin::new(42);
        let scale = 0.08;

        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| perlin.get([x as f64 * scale, y as f64 * scale]))
                    .collect()
            })
            .collect()
    }

    fn run(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
        while !self.exit {
            terminal.draw(|frame: &mut ratatui::Frame<'_>| self.draw(frame))?;

            // Read events AFTER drawing so the first frame is visible immediately
            match crossterm::event::read()? {
                crossterm::event::Event::Key(key_event) => self.handle_key_event(key_event)?,
                _ => {}
            }
        }

        Ok(())
    }

    fn draw(&self, frame: &mut ratatui::Frame<'_>) {
        frame.render_widget(self, frame.area());
    }

    fn handle_key_event(&mut self, key_event: crossterm::event::KeyEvent) -> io::Result<()> {
        if key_event.kind == KeyEventKind::Press {
            self.exit = true;
        }
        Ok(())
    }
}

impl Widget for &App {
    fn render(self, area: ratatui::prelude::Rect, buf: &mut ratatui::prelude::Buffer)
    where
        Self: Sized,
    {
        //Render le bruit de Perlin sur toute la carte
        for y in 0..area.height as usize {
            for x in 0..area.width as usize {
                
                //Evite de dessiner le bruit sur la base
                if is_base_cell(x as u16, y as u16, self.base_pos) {
                    continue;
                }

                //Evite de dessiner le bruit sur les bords de la carte
                if x == 0 || x == area.width as usize - 1 || y == 0 || y == area.height as usize - 1 {
                    continue;
                }

                //Todo : Creer un nombre random pour les cristaux et l'energie 
                //Passer le compteur restant de ressources dans la fonction render_cell 
                //Render les cristaux et l'energie en fonction du compteur restant


                let map_y = y % self.height;
                let map_x = x % self.width;
                let value = self.map[map_y][map_x];

                let (symbol, color) = render_cell(value);

                buf[(area.x + x as u16, area.y + y as u16)]
                    .set_symbol(symbol)
                    .set_style(Style::default().fg(color));
            }
        }

        // Base au milieu de la carte
        render_base(self.base_pos, area, buf);

        // Title overlay on the first line
        Line::from("Robots Game — Appuyez sur n'importe quelle touche pour quitter")
            .bold().yellow()
            .render(area, buf);

        //Render les robots
        for robot in &self.robots {
            let (rx, ry) = robot.position;
            if rx < area.width && ry < area.height {
                buf[(area.x + rx, area.y + ry)]
                    .set_symbol("X")
                    .set_style(Style::default().fg(Color::LightRed).bold());
            }
        }
    }
}

fn render_cell(value: f64) -> (&'static str, Color) {
    
    
    match value {
    
        v if v < -0.1 => ("0", Color::Cyan),
        v if v <  0.0 => (".", Color::Yellow),
        v if v <  0.5 => (" ", Color::DarkGray),
        _             => ("", Color::White),
    }
}

fn render_base(pos: (u16, u16), area: Rect, buf: &mut ratatui::prelude::Buffer) {
    // The base is a 3x3 grid of cells:
    //  ┌─┐
    //  │B│
    //  └─┘
    let tiles: &[(&str, &str, &str)] = &[
        ("#", "#", "#"),
        ("#", "#", "#"),
        ("#", "#", "#"),
    ];

    let base_style = Style::default().fg(Color::Green).bold();

    for (row, (left, center, right)) in tiles.iter().enumerate() {
        let y = area.y + pos.1 + row as u16 - 1; // -1 to center the 3 rows on pos
        let x = area.x + pos.0;

        // Bounds check — don't draw outside the terminal area
        if y < area.y || y >= area.y + area.height {
            continue;
        }

        for (col, symbol) in [left, center, right].iter().enumerate() {
            let cx = x + col as u16 - 1; // -1 to center the 3 cols on pos
            if cx < area.x || cx >= area.x + area.width {
                continue;
            }
            buf[(cx, y)].set_symbol(symbol).set_style(base_style);
        }
    }
}

fn is_base_cell(x: u16, y: u16, pos: (u16, u16)) -> bool {
    let (bx, by) = pos;
    //Valeur pour une base 3x3, changer si la taille de la base change
    let x_min = bx.saturating_sub(2);
    let y_min = by.saturating_sub(2);
    let x_max = bx + 5;
    let y_max = by + 5;

    x >= x_min && x <= x_max && y >= y_min && y <= y_max
}