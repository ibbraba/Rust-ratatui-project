use std::io::{self, Stdout};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use std::collections::{HashMap, HashSet, VecDeque};
use crossterm::event::KeyEventKind;
use noise::{NoiseFn, Perlin};
use rand::Rng;
use ratatui::backend::CrosstermBackend;
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::Line;
use ratatui::widgets::Widget;
use ratatui::Terminal;
use ratatui::prelude::Rect;
use rand::seq::SliceRandom;


fn main() -> io::Result<()> {
    let mut terminal: Terminal<CrosstermBackend<Stdout>> = ratatui::init();

    let size = terminal.size()?;
    let mut app = App::new(size.width as usize, size.height as usize);

    let app_result = app.run(&mut terminal);

    ratatui::restore();
    app_result
}

/// État partagé entre les threads robots (protégé par Mutex).
struct SimulationState {
    map: Arc<Vec<Vec<f64>>>,
    width: usize,
    height: usize,
    base_pos: (u16, u16),
    robots: Vec<Robot>,
    collected_crystals: HashSet<(u16, u16)>,
    collected_energy: HashSet<(u16, u16)>,
    deposited_crystals: u32,
    deposited_energy: u32,
    discovered_crystals: HashSet<(u16, u16)>,
    discovered_energy: HashSet<(u16, u16)>,
    resource_quantities: HashMap<(u16, u16), u32>,
}

pub struct App {
    exit: bool,
    state: Arc<Mutex<SimulationState>>,
    last_tick: Instant,
    tick_rate: Duration,
    tick_senders: Vec<Sender<()>>,
    done_receiver: Receiver<()>,
    stop: Arc<AtomicBool>,
    robot_handles: Vec<JoinHandle<()>>,
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
    pub carried_energy: u32,
    pub path: VecDeque<(u16, u16)>,
    pub preferred_dir: (i16, i16),
}

impl Robot {
    pub fn new(position: (u16, u16), robot_type: RobotType, preferred_dir: (i16, i16)) -> Self {
        Self {
            position,
            robot_type,
            // Les robots commencent en mode exploration par défaut
            state: RobotState::Exploring,
            carried_crystals: 0,
            carried_energy: 0,
            path: VecDeque::new(),
            preferred_dir: (0, 0),
        }
    }
    
}

impl App {
    pub fn new(width: usize, height: usize) -> Self {
        let base_pos = (width as u16 / 2, height as u16 / 2);

        let spawn = |dx: i16, dy: i16| -> (u16, u16) {
        let x = (base_pos.0 as i16 + dx).clamp(1, width as i16 - 2) as u16;
        let y = (base_pos.1 as i16 + dy).clamp(1, height as i16 - 2) as u16;
        (x, y)
    };
        let map = Self::generate_map(width, height);

        let mut resource_quantities = HashMap::new();
        let mut rng = rand::thread_rng();
        for y in 0..height as u16 {
            for x in 0..width as u16 {
                if is_base_cell(x, y, base_pos) || is_obstacle(&map, x, y) {
                    continue;
                }
                if is_crystal(&map, x, y) || is_energy(&map, x, y) {
                    resource_quantities.insert((x, y), rng.gen_range(50..=250));
                }
            }
        }

        let robots = vec![
            Robot::new(spawn(12, 0), RobotType::Scout, (1, 0)),
            Robot::new(spawn(0, -12), RobotType::Scout, (0, -1)),
            Robot::new(spawn(-12, 0), RobotType::Scout, (-1, 0)),
            Robot::new(spawn(0, 12), RobotType::Scout, (0, 1)),
            Robot::new(spawn(12, 2), RobotType::Collector, (1, 0)),
            Robot::new(spawn(12, -2), RobotType::Collector, (1, 0)),
            Robot::new(spawn(-12, 2), RobotType::Collector, (-1, 0)),
            Robot::new(spawn(-12, -2), RobotType::Collector, (-1, 0)),
        ];

        let map = Arc::new(map);
        let stop = Arc::new(AtomicBool::new(false));
        let (done_tx, done_receiver) = mpsc::channel();

        let state = Arc::new(Mutex::new(SimulationState {
            map: Arc::clone(&map),
            width,
            height,
            base_pos,
            robots,
            collected_crystals: HashSet::new(),
            collected_energy: HashSet::new(),
            deposited_crystals: 0,
            deposited_energy: 0,
            discovered_crystals: HashSet::new(),
            discovered_energy: HashSet::new(),
            resource_quantities,
        }));

        let mut tick_senders = Vec::new();
        let mut robot_handles = Vec::new();

        for robot_id in 0..state.lock().unwrap().robots.len() {
            let (tick_tx, tick_rx) = mpsc::channel();
            tick_senders.push(tick_tx);

            let thread_state = Arc::clone(&state);
            let thread_stop = Arc::clone(&stop);
            let thread_done = done_tx.clone();

            robot_handles.push(thread::spawn(move || {
                robot_thread_loop(robot_id, thread_state, tick_rx, thread_done, thread_stop);
            }));
        }

        drop(done_tx);

        Self {
            exit: false,
            state,
            last_tick: Instant::now(),
            tick_rate: Duration::from_millis(100),
            tick_senders,
            done_receiver,
            stop,
            robot_handles,
        }
    }

    fn generate_map(width: usize, height: usize) -> Vec<Vec<f64>> {
        let perlin = Perlin::new(42);
        let scale = 0.1;
        let base_x = width as i32 / 2;
        let base_y = height as i32 / 2;

        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| {
                        let dx = x as i32 - base_x;
                        let dy = y as i32 - base_y;
                        if dx.abs().max(dy.abs()) <= 15 {
                            0.0 // Empty ground, no obstacle, crystal, or energy
                        } else {
                            perlin.get([x as f64 * scale, y as f64 * scale])
                        }
                    })
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

        // Envoie un tick à chaque thread robot (simulation concurrente)
        if self.last_tick.elapsed() >= self.tick_rate {
            self.signal_robot_tick();
            self.last_tick = Instant::now();
        }
    }

        self.shutdown();
        Ok(())
    }

    fn signal_robot_tick(&self) {
        let robot_count = self.tick_senders.len();

        for tx in &self.tick_senders {
            let _ = tx.send(());
        }

        for _ in 0..robot_count {
            let _ = self.done_receiver.recv();
        }

        if let Ok(mut state) = self.state.lock() {
            assign_collectors(&mut state);
        }
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);

        for tx in &self.tick_senders {
            let _ = tx.send(());
        }

        for handle in self.robot_handles.drain(..) {
            let _ = handle.join();
        }
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

/// Boucle exécutée dans un thread dédié par robot.
fn robot_thread_loop(
    robot_id: usize,
    state: Arc<Mutex<SimulationState>>,
    tick_rx: Receiver<()>,
    done_tx: Sender<()>,
    stop: Arc<AtomicBool>,
) {
    while !stop.load(Ordering::Relaxed) {
        if tick_rx.recv().is_err() {
            break;
        }

        if stop.load(Ordering::Relaxed) {
            break;
        }

        let mut rng = rand::thread_rng();
        if let Ok(mut sim) = state.lock() {
            tick_robot(&mut sim, robot_id, &mut rng);
        }

        let _ = done_tx.send(());
    }
}

fn tick_robot(state: &mut SimulationState, robot_id: usize, rng: &mut rand::rngs::ThreadRng) {
    let map = Arc::clone(&state.map);
    let width = state.width;
    let height = state.height;
    let base_pos = state.base_pos;
    let robot_type = state.robots[robot_id].robot_type;

    match robot_type {
        RobotType::Scout => {
            {
                let robot = &mut state.robots[robot_id];
                move_scout(robot, &map, width, height, base_pos, rng);
            }

            let (rx, ry) = state.robots[robot_id].position;

            if is_crystal(&map, rx, ry)
                && !state.collected_crystals.contains(&(rx, ry))
                && !state.discovered_crystals.contains(&(rx, ry))
            {
                state.discovered_crystals.insert((rx, ry));
            }

            if is_energy(&map, rx, ry)
                && !state.collected_energy.contains(&(rx, ry))
                && !state.discovered_energy.contains(&(rx, ry))
            {
                state.discovered_energy.insert((rx, ry));
            }
        }

        RobotType::Collector => {
            let next = state.robots[robot_id].path.pop_front();

            if let Some(next) = next {
                state.robots[robot_id].position = next;

                if next == base_pos {
                    let (crystals, energy) = {
                        let robot = &state.robots[robot_id];
                        (robot.carried_crystals, robot.carried_energy)
                    };
                    if crystals > 0 || energy > 0 {
                        state.deposited_crystals += crystals;
                        state.deposited_energy += energy;
                        let robot = &mut state.robots[robot_id];
                        robot.carried_crystals = 0;
                        robot.carried_energy = 0;
                        robot.state = RobotState::Exploring;

                        let exits: &[(i16, i16)] = &[(2, 0), (-2, 0), (0, 2), (0, -2)];
                        for (dx, dy) in exits {
                            let ex = (next.0 as i16 + dx).clamp(1, width as i16 - 2) as u16;
                            let ey = (next.1 as i16 + dy).clamp(1, height as i16 - 2) as u16;
                            if !is_base_cell(ex, ey, base_pos) && !is_obstacle(&map, ex, ey) {
                                robot.path = VecDeque::from([(ex, ey)]);
                                break;
                            }
                        }
                    }
                }

                let robot_state = state.robots[robot_id].state;
                let collected_c = state.collected_crystals.clone();
                let collected_e = state.collected_energy.clone();

                if is_crystal(&map, next.0, next.1)
                    && !collected_c.contains(&next)
                    && robot_state != RobotState::ReturningToBase
                {
                    state.collected_crystals.insert(next);
                    let path = bfs(
                        &map,
                        &state.collected_crystals,
                        &state.collected_energy,
                        next,
                        base_pos,
                        width,
                        height,
                        base_pos,
                    );
                    let robot = &mut state.robots[robot_id];
                    robot.carried_crystals += 1;
                    robot.path = path;
                    robot.state = RobotState::ReturningToBase;
                } else if is_energy(&map, next.0, next.1)
                    && !collected_e.contains(&next)
                    && robot_state != RobotState::ReturningToBase
                {
                    state.collected_energy.insert(next);
                    let path = bfs(
                        &map,
                        &state.collected_crystals,
                        &state.collected_energy,
                        next,
                        base_pos,
                        width,
                        height,
                        base_pos,
                    );
                    let robot = &mut state.robots[robot_id];
                    robot.carried_energy += 1;
                    robot.path = path;
                    robot.state = RobotState::ReturningToBase;
                }
            } else {
                {
                    let robot = &mut state.robots[robot_id];
                    robot.state = RobotState::Exploring;
                    move_scout(robot, &map, width, height, base_pos, rng);
                }

                let (rx, ry) = state.robots[robot_id].position;

                if is_crystal(&map, rx, ry) && !state.collected_crystals.contains(&(rx, ry)) {
                    state.collected_crystals.insert((rx, ry));
                    let path = bfs(
                        &map,
                        &state.collected_crystals,
                        &state.collected_energy,
                        (rx, ry),
                        base_pos,
                        width,
                        height,
                        base_pos,
                    );
                    let robot = &mut state.robots[robot_id];
                    robot.carried_crystals += 1;
                    robot.path = path;
                    robot.state = RobotState::ReturningToBase;
                } else if is_energy(&map, rx, ry) && !state.collected_energy.contains(&(rx, ry)) {
                    state.collected_energy.insert((rx, ry));
                    let path = bfs(
                        &map,
                        &state.collected_crystals,
                        &state.collected_energy,
                        (rx, ry),
                        base_pos,
                        width,
                        height,
                        base_pos,
                    );
                    let robot = &mut state.robots[robot_id];
                    robot.carried_energy += 1;
                    robot.path = path;
                    robot.state = RobotState::ReturningToBase;
                }
            }
        }
    }
}

fn assign_collectors(state: &mut SimulationState) {
    let mut already_targeted: HashSet<(u16, u16)> = state
        .robots
        .iter()
        .filter(|r| r.robot_type == RobotType::Collector && !r.path.is_empty())
        .filter_map(|r| r.path.back().copied())
        .collect();

    let available_crystals = state
        .discovered_crystals
        .iter()
        .filter(|p| !state.collected_crystals.contains(p) && !already_targeted.contains(p))
        .copied();

    let available_energy = state
        .discovered_energy
        .iter()
        .filter(|p| !state.collected_energy.contains(p) && !already_targeted.contains(p))
        .copied();

    let mut all_targets: Vec<(u16, u16)> = available_crystals.chain(available_energy).collect();

    for robot in &mut state.robots {
        if robot.robot_type != RobotType::Collector
            || !robot.path.is_empty()
            || robot.carried_crystals > 0
            || robot.carried_energy > 0
        {
            continue;
        }

        let best_placed = all_targets
            .iter()
            .enumerate()
            .min_by_key(|(_, pos)| {
                let dx = pos.0 as i32 - robot.position.0 as i32;
                let dy = pos.1 as i32 - robot.position.1 as i32;
                dx * dx + dy * dy
            })
            .map(|(i, pos)| (i, *pos));

        if let Some((idx, target)) = best_placed {
            let path = bfs(
                &state.map,
                &state.collected_crystals,
                &state.collected_energy,
                robot.position,
                target,
                state.width,
                state.height,
                state.base_pos,
            );
            if !path.is_empty() {
                robot.path = path;
                robot.state = RobotState::Collecting;
                all_targets.remove(idx);
                already_targeted.insert(target);
            }
        }
    }
}

impl Widget for &App {
    fn render(self, area: ratatui::prelude::Rect, buf: &mut ratatui::prelude::Buffer)
    where
        Self: Sized,
    {
        let state = match self.state.lock() {
            Ok(s) => s,
            Err(_) => return,
        };

        for y in 0..area.height as usize {
            for x in 0..area.width as usize {
                if is_base_cell(x as u16, y as u16, state.base_pos) {
                    continue;
                }

                if x == 0 || x == area.width as usize - 1 || y == 0 || y == area.height as usize - 1
                {
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
        v if v <  0.15 => (" ", Color::DarkGray), // Decolle les cristaux des obstacles 
        v if v <  0.30 => ("C", Color::Yellow),
        v if v <  0.45 => (" ", Color::DarkGray),
        v if v <  0.60 => ("E", Color::Green),
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
    let x_min = bx.saturating_sub(1);
    let y_min = by.saturating_sub(1);
    let x_max = bx + 5;
    let y_max = by + 5;

    x >= x_min && x <= x_max && y >= y_min && y <= y_max
}

fn map_value(map: &[Vec<f64>], x: u16, y: u16) -> f64 {
    map[y as usize % map.len()][x as usize % map[0].len()]
}

fn is_obstacle(map: &[Vec<f64>], x: u16, y: u16) -> bool {
    map_value(map, x, y) < -0.1
}

fn is_crystal(map: &[Vec<f64>], x: u16, y: u16) -> bool {
    let v = map_value(map, x, y);
    v >= 0.15 && v < 0.30
}

fn is_energy(map: &[Vec<f64>], x: u16, y: u16) -> bool {
    let v = map_value(map, x, y);
    v >= 0.45 && v < 0.60
}

fn move_scout(
    robot: &mut Robot,
    map: &Arc<Vec<Vec<f64>>>,
    width: usize,
    height: usize,
    base_pos: (u16, u16),
    rng: &mut rand::rngs::ThreadRng,
) {
    let directions = [(0i16,-1i16), (0,1), (-1,0), (1,0)];

    // Build candidate list: preferred dir first, then random others
    let mut candidates = vec![robot.preferred_dir];
    let mut others: Vec<(i16, i16)> = directions.iter()
        .filter(|&&d| d != robot.preferred_dir)
        .copied()
        .collect();
    others.shuffle(rng);
    candidates.extend(others);

    // 70% chance to try preferred dir first, 30% go fully random
    if rng.gen_bool(0.30) {
        candidates.shuffle(rng);
    }

    for (dx, dy) in candidates {
        let nx = (robot.position.0 as i16 + dx).clamp(1, width as i16 - 2) as u16;
        let ny = (robot.position.1 as i16 + dy).clamp(1, height as i16 - 2) as u16;

        if !is_obstacle(map, nx, ny) && !is_base_cell(nx, ny, base_pos) {
            robot.position = (nx, ny);

            // Occasionally rotate preferred direction to avoid getting stuck
            if rng.gen_bool(0.05) {
                robot.preferred_dir = directions[rng.gen_range(0..4)];
            }
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
    map: &Arc<Vec<Vec<f64>>>,
    collected_crystals: &HashSet<(u16, u16)>,
    collected_energy: &HashSet<(u16, u16)>,   // <-- add
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


