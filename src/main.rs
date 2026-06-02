use std::io::{self, Stdout};
use std::time::{Instant, Duration};
use std::collections::{HashSet, VecDeque, HashMap};
use crossterm::event::KeyEventKind;
use noise::{NoiseFn, Perlin};
use rand::Rng;
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
    last_tick: Instant,       
    tick_rate: Duration,      // <-- add
    collected_crystals: HashSet<(u16, u16)>,  // <-- add
    discovered_crystals: HashSet<(u16, u16)>, // Cristaux découverts par les scouts, base de savoir de tous les robots
}

#[derive(Clone, Copy, PartialEq)]
pub enum RobotType {
    Scout,
    Collector,
}

#[derive(Clone, Copy, PartialEq)]
pub enum RobotState {
    Exploring,
    Collecting,
    ReturningToBase,
    Idle,
}

pub struct Robot {
    pub position: (u16, u16),
    pub robot_type: RobotType,
    pub state: RobotState,
    pub carried_crystals: u32,
    pub path: VecDeque<(u16, u16)>,
}

impl Robot {
    pub fn new(position: (u16, u16), robot_type: RobotType) -> Self {
        Self {
            position,
            robot_type,
            // Les robots commencent en mode exploration par défaut
            state: RobotState::Exploring,
            carried_crystals: 0,
            path: VecDeque::new(),

        }
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
            robots: vec![
                Robot::new((base_pos.0 + 5, base_pos.1), RobotType::Scout),
                Robot::new((base_pos.0 + 7, base_pos.1), RobotType::Scout),

                Robot::new((base_pos.0 - 5, base_pos.1), RobotType::Collector),
                Robot::new((base_pos.0 - 7, base_pos.1), RobotType::Collector),
            ], // Example robot starting near the base
            last_tick: Instant::now(),
            tick_rate: Duration::from_millis(200), // 100 ms per tick
            collected_crystals: HashSet::new(),
            discovered_crystals: HashSet::new(),
        }
    }

    fn generate_map(width: usize, height: usize) -> Vec<Vec<f64>> {
        let perlin = Perlin::new(42);
        let scale = 0.09;

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

        // Temps restant avant le prochain tick
        let timeout = self.tick_rate
            .checked_sub(self.last_tick.elapsed())
            .unwrap_or(Duration::ZERO);

        // poll() bloque AU MAXIMUM jusqu'au prochain tick, puis rend la main
        if crossterm::event::poll(timeout)? {
            match crossterm::event::read()? {
                crossterm::event::Event::Key(key_event) => self.handle_key_event(key_event)?,
                _ => {}
            }
        }

        // Déplacer les robots à chaque tick
        if self.last_tick.elapsed() >= self.tick_rate {
            self.update_robots();
            self.last_tick = Instant::now();
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

  fn update_robots(&mut self) {
    let mut rng = rand::thread_rng();
    let mut newly_discovered: Vec<(u16, u16)> = vec![];

    for robot in &mut self.robots {
        match robot.robot_type {

            RobotType::Scout => {
                // Move randomly, report any crystal found
                move_scout(robot, &self.map, self.width, self.height, self.base_pos, &mut rng);

                let (rx, ry) = robot.position;
                if is_crystal(&self.map, rx, ry)
                    && !self.collected_crystals.contains(&(rx, ry))
                    && !self.discovered_crystals.contains(&(rx, ry))
                {
                    newly_discovered.push((rx, ry));
                }
            }

            RobotType::Collector => {
                if let Some(next) = robot.path.pop_front() {
                    robot.position = next;

                    // Collect any crystal stepped on, whether it was the target or not
                    if is_crystal(&self.map, next.0, next.1)
                        && !self.collected_crystals.contains(&next)
                    {
                        robot.carried_crystals += 1;
                        self.collected_crystals.insert(next);
                        self.discovered_crystals.remove(&next);

                        // If this was the target (path now empty), go back to exploring
                        // If not the target, keep following the path to the original target
                        if robot.path.is_empty() {
                            robot.state = RobotState::ReturningToBase;
                        }
                    }
                } else {
                    robot.state = RobotState::Exploring;
                    move_scout(robot, &self.map, self.width, self.height, self.base_pos, &mut rng);

                    // Also collect while wandering randomly
                    let (rx, ry) = robot.position;
                    if is_crystal(&self.map, rx, ry)
                        && !self.collected_crystals.contains(&(rx, ry))
                    {
                        robot.carried_crystals += 1;
                        self.collected_crystals.insert((rx, ry));
                        self.discovered_crystals.remove(&(rx, ry));
                    }
                }
            }
        }
    }

    // Register newly found crystals
    for pos in newly_discovered {
        self.discovered_crystals.insert(pos);
    }

    // Assign unoccupied collectors to known crystals
    self.assign_collectors();
    }

    fn assign_collectors(&mut self) {
    // Build list of crystals not yet targeted by a collector
    let targeted: HashSet<(u16, u16)> = self.robots.iter()
        .filter(|r| r.robot_type == RobotType::Collector && !r.path.is_empty())
        .filter_map(|r| r.path.back().copied())
        .collect();

    let available_crystals: Vec<(u16, u16)> = self.discovered_crystals
        .iter()
        .filter(|pos| !self.collected_crystals.contains(pos) && !targeted.contains(pos))
        .copied()
        .collect();

    for robot in &mut self.robots {
        if robot.robot_type != RobotType::Collector || !robot.path.is_empty() {
            continue;
        }

        // Find the closest available crystal
        if let Some(&target) = available_crystals.iter().min_by_key(|&&(cx, cy)| {
            let dx = cx as i32 - robot.position.0 as i32;
            let dy = cy as i32 - robot.position.1 as i32;
            dx * dx + dy * dy
        }) {
            let path = bfs(
                &self.map,
                &self.collected_crystals,
                robot.position,
                target,
                self.width,
                self.height,
                self.base_pos,
            );
            if !path.is_empty() {
                robot.path = path;
                robot.state = RobotState::Collecting;
            }
        }
    }
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


                if y >= self.height || x >= self.width {
                    continue;
                }

                let value = self.map[y][x];

               let (symbol, color) = if self.collected_crystals.contains(&(x as u16, y as u16)) {
                    (" ", Color::DarkGray)
                } else if self.discovered_crystals.contains(&(x as u16, y as u16)) {
                    ("C", Color::LightYellow)  // brighter = known to all robots
                } else {
                    render_cell(value)
                };

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
            let (symbol, color) = match robot.robot_type {
                RobotType::Scout => ("X", Color::LightRed),
                RobotType::Collector => ("o", Color::Magenta),
            };

            buf[(area.x + rx, area.y + ry)]
                .set_symbol(symbol)
                .set_style(Style::default().fg(color).bold());
            }
    }
}

fn render_cell(value: f64) -> (&'static str, Color) {
    
    
    match value {
    
        v if v < -0.1 => ("O", Color::Cyan),
        v if v <  0.0 => ("C", Color::Yellow),
        v if v <  0.5 => (" ", Color::DarkGray),
        _             => (" ", Color::White),
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

fn map_value(map: &Vec<Vec<f64>>, x: u16, y: u16) -> f64 {
    let map_y = y as usize % map.len();
    let map_x = x as usize % map[0].len();
    map[map_y][map_x]
}

fn is_obstacle(map: &Vec<Vec<f64>>, x: u16, y: u16) -> bool {
    map_value(map, x, y) < -0.1
}

fn is_crystal(map: &Vec<Vec<f64>>, x: u16, y: u16) -> bool {
    let v = map_value(map, x, y);
    v >= -0.1 && v < 0.0
}


fn move_scout(
    robot: &mut Robot,
    map: &Vec<Vec<f64>>,
    width: usize,
    height: usize,
    base_pos: (u16, u16),
    rng: &mut rand::rngs::ThreadRng,
) {
    let directions = [(0,-1), (0,1), (-1,0), (1,0)];

    for _ in 0..4 {
        let (dx, dy) = directions[rng.gen_range(0..4)];

        let nx = (robot.position.0 as i16 + dx)
            .clamp(1, width as i16 - 2) as u16;

        let ny = (robot.position.1 as i16 + dy)
            .clamp(1, height as i16 - 2) as u16;

        if !is_obstacle(map, nx, ny)
            && !is_base_cell(nx, ny, base_pos)
        {
            robot.position = (nx, ny);
            break;
        }
    }
}


fn move_collector(
    robot: &mut Robot,
    map: &Vec<Vec<f64>>,
    width: usize,
    height: usize,
    base_pos: (u16, u16),
    rng: &mut rand::rngs::ThreadRng,
) -> Option<(u16, u16)> {
    let directions = [(0,-1), (0,1), (-1,0), (1,0)];

    for _ in 0..4 {
        let (dx, dy) = directions[rng.gen_range(0..4)];

        let nx = (robot.position.0 as i16 + dx)
            .clamp(1, width as i16 - 2) as u16;

        let ny = (robot.position.1 as i16 + dy)
            .clamp(1, height as i16 - 2) as u16;

        if !is_obstacle(map, nx, ny)
            && !is_base_cell(nx, ny, base_pos)
        {
            robot.position = (nx, ny);

            if is_crystal(map, nx, ny) {
                robot.carried_crystals += 1;
                return Some((nx, ny));
            }

            break;
        }
    }
    None
}

fn bfs(
    map: &Vec<Vec<f64>>,
    collected: &HashSet<(u16, u16)>,
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

        for (dx, dy) in [(0i16,-1),(0,1),(-1,0),(1,0)] {
            let nx = (x as i16 + dx).clamp(1, width as i16 - 2) as u16;
            let ny = (y as i16 + dy).clamp(1, height as i16 - 2) as u16;

            if visited.contains(&(nx, ny)) {
                continue;
            }
            // Allow passage through collected crystal spots (now empty ground)
            let passable = !is_obstacle(map, nx, ny)
                && (!is_base_cell(nx, ny, base_pos) || (nx, ny) == to)
                && !collected.contains(&(nx, ny)).then(|| false).unwrap_or(false);

            if passable {
                visited.insert((nx, ny));
                let mut new_path = path.clone();
                new_path.push((nx, ny));
                queue.push_back((nx, ny, new_path));
            }
        }
    }

    VecDeque::new() // no path found
}

