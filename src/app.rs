use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{self, Stdout};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crossterm::event::KeyEventKind;
use noise::{NoiseFn, Perlin};
use rand::Rng;
use rand::seq::SliceRandom;
use ratatui::backend::CrosstermBackend;
use ratatui::prelude::Rect;
use ratatui::widgets::Widget;
use ratatui::Terminal;

use crate::pathfinding::bfs;
use crate::render;
use crate::world::{is_base_cell, is_crystal, is_energy, is_obstacle};

/// État partagé entre les threads robots (protégé par Mutex).
pub(crate) struct SimulationState {
    pub(crate) map: Arc<Vec<Vec<f64>>>,
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) base_pos: (u16, u16),
    pub(crate) robots: Vec<Robot>,
    pub(crate) collected_crystals: HashSet<(u16, u16)>,
    pub(crate) collected_energy: HashSet<(u16, u16)>,
    pub(crate) deposited_crystals: u32,
    pub(crate) deposited_energy: u32,
    pub(crate) discovered_crystals: HashSet<(u16, u16)>,
    pub(crate) discovered_energy: HashSet<(u16, u16)>,
    pub(crate) resource_quantities: HashMap<(u16, u16), u32>,
}

pub(crate) struct App {
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
pub(crate) enum RobotType {
    Scout,
    Collector,
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum RobotState {
    Exploring,
    Collecting,
    ReturningToBase,
    Idle,
}

pub(crate) struct Robot {
    pub(crate) position: (u16, u16),
    pub(crate) robot_type: RobotType,
    pub(crate) state: RobotState,
    pub(crate) carried_crystals: u32,
    pub(crate) carried_energy: u32,
    pub(crate) path: VecDeque<(u16, u16)>,
    pub(crate) preferred_dir: (i16, i16),
}

impl Robot {
    pub fn new(position: (u16, u16), robot_type: RobotType, preferred_dir: (i16, i16)) -> Self {
        Self {
            position,
            robot_type,
            state: RobotState::Exploring,
            carried_crystals: 0,
            carried_energy: 0,
            path: VecDeque::new(),
            preferred_dir,
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

        let mut spawn_positions = Vec::new();
        for dy in -1..=1 {
            for dx in -1..=1 {
                let x = (base_pos.0 as i16 + dx).clamp(1, width as i16 - 2) as u16;
                let y = (base_pos.1 as i16 + dy).clamp(1, height as i16 - 2) as u16;
                spawn_positions.push((x, y));
            }
        }

        // Assigner une position unique de la liste à chaque robot (8 robots pour 9 cases)
        let robots = vec![
            Robot::new(spawn_positions[0], RobotType::Scout, (1, 0)),
            Robot::new(spawn_positions[1], RobotType::Scout, (0, -1)),
            Robot::new(spawn_positions[2], RobotType::Scout, (-1, 0)),
            Robot::new(spawn_positions[3], RobotType::Scout, (0, 1)),
            Robot::new(spawn_positions[4], RobotType::Collector, (1, 0)),
            Robot::new(spawn_positions[5], RobotType::Collector, (1, 0)),
            Robot::new(spawn_positions[6], RobotType::Collector, (-1, 0)),
            Robot::new(spawn_positions[7], RobotType::Collector, (-1, 0)),
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

        let robot_count = state.lock().unwrap().robots.len();
        for robot_id in 0..robot_count {
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
                            0.0
                        } else {
                            perlin.get([x as f64 * scale, y as f64 * scale])
                        }
                    })
                    .collect()
            })
            .collect()
    }

    pub fn run(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
        while !self.exit {
            terminal.draw(|frame: &mut ratatui::Frame<'_>| self.draw(frame))?;

            let timeout = self
                .tick_rate
                .checked_sub(self.last_tick.elapsed())
                .unwrap_or(Duration::ZERO);

            if crossterm::event::poll(timeout)? {
                match crossterm::event::read()? {
                    crossterm::event::Event::Key(key_event) => self.handle_key_event(key_event)?,
                    _ => {}
                }
            }

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

impl Widget for &App {
    fn render(self, area: Rect, buf: &mut ratatui::prelude::Buffer)
    where
        Self: Sized,
    {
        let state = match self.state.lock() {
            Ok(s) => s,
            Err(_) => return,
        };

        render::render_world(&state, area, buf);
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

fn move_scout(
    robot: &mut Robot,
    map: &Arc<Vec<Vec<f64>>>,
    width: usize,
    height: usize,
    base_pos: (u16, u16),
    rng: &mut rand::rngs::ThreadRng,
) {
    let directions = [(0i16, -1i16), (0, 1), (-1, 0), (1, 0)];

    let mut candidates = vec![robot.preferred_dir];
    let mut others: Vec<(i16, i16)> = directions
        .iter()
        .filter(|&&d| d != robot.preferred_dir)
        .copied()
        .collect();
    others.shuffle(rng);
    candidates.extend(others);

    if rng.gen_bool(0.30) {
        candidates.shuffle(rng);
    }

    for (dx, dy) in candidates {
        let nx = (robot.position.0 as i16 + dx).clamp(1, width as i16 - 2) as u16;
        let ny = (robot.position.1 as i16 + dy).clamp(1, height as i16 - 2) as u16;

        if !is_obstacle(map, nx, ny) && !is_base_cell(nx, ny, base_pos) {
            robot.position = (nx, ny);

            if rng.gen_bool(0.05) {
                robot.preferred_dir = directions[rng.gen_range(0..4)];
            }
            break;
        }
    }
}
