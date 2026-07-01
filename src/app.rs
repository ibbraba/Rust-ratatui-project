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

impl SimulationState {
    #[cfg(test)]
    pub(crate) fn new_test(map: Vec<Vec<f64>>, base_pos: (u16, u16), robots: Vec<Robot>) -> Self {
        let width = map[0].len();
        let height = map.len();
        Self {
            map: Arc::new(map),
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
            resource_quantities: HashMap::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn tick(&mut self, rng: &mut impl Rng) {
        for robot_id in 0..self.robots.len() {
            tick_robot(self, robot_id, rng);
        }
        assign_collectors(self);
    }
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

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum RobotType {
    Scout,
    Collector,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum RobotState {
    Exploring,
    MovingToResource,
    Collecting,
    ReturningToBase,
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
        let map = Self::generate_map(width, height);
        let mut resource_quantities = HashMap::new();
        let mut rng = rand::thread_rng();

        for y in 0..height as u16 {
            for x in 0..width as u16 {
                if is_base_cell(x, y, base_pos) || is_obstacle(&map, x, y) {
                    continue;
                }

                if is_crystal(&map, x, y) || is_energy(&map, x, y) {
                    // RARIFICATION : Seulement 10% des ressources éligibles sont créées
                    if rng.gen_bool(0.10) {
                        let qty = rng.gen_range(50..=200);
                        resource_quantities.insert((x, y), qty);
                    }
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
            tick_rate: Duration::from_millis(25), // Simulation rapide
            tick_senders,
            done_receiver,
            stop,
            robot_handles,
        }
    }

    fn generate_map(width: usize, height: usize) -> Vec<Vec<f64>> {
        let perlin = Perlin::new(7);
        let scale = 0.1;
        let base_x = width as i32 / 2;
        let base_y = height as i32 / 2;

        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| {
                        let dx = x as i32 - base_x;
                        let dy = y as i32 - base_y;
                        if dx.abs().max(dy.abs()) <= 3 {
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
                if let crossterm::event::Event::Key(key_event) = crossterm::event::read()? {
                    self.handle_key_event(key_event)?;
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
    fn render(self, area: Rect, buf: &mut ratatui::prelude::Buffer) {
        if let Ok(state) = self.state.lock() {
            render::render_world(&state, area, buf);
        }
    }
}

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

/// Tries to find a new BFS path to `target` and moves the robot one step along it.
/// Returns `true` if a new path was found and the robot advanced.
fn reroute_path(state: &mut SimulationState, robot_id: usize, target: (u16, u16)) -> bool {
    let current_pos = state.robots[robot_id].position;
    let new_path = bfs(
        &state.map,
        &state.collected_crystals,
        &state.collected_energy,
        current_pos,
        target,
        state.width,
        state.height,
        state.base_pos,
    );
    if !new_path.is_empty() {
        state.robots[robot_id].path = new_path;
        if let Some(new_next) = state.robots[robot_id].path.pop_front() {
            state.robots[robot_id].position = new_next;
        }
        true
    } else {
        false
    }
}

/// Tries to move the robot one cell sideways to avoid a blockage.
/// Returns `true` if a valid sidestep was found and applied.
fn step_sideways(
    state: &mut SimulationState,
    robot_id: usize,
    occupied_positions: &HashSet<(u16, u16)>,
) -> bool {
    let current_pos = state.robots[robot_id].position;
    let width = state.width;
    let height = state.height;
    let base_pos = state.base_pos;
    let map = Arc::clone(&state.map);

    for (dx, dy) in [(0i16, -1i16), (0, 1), (-1, 0), (1, 0)] {
        let nx = (current_pos.0 as i16 + dx).clamp(1, width as i16 - 2) as u16;
        let ny = (current_pos.1 as i16 + dy).clamp(1, height as i16 - 2) as u16;
        if !is_obstacle(&map, nx, ny)
            && !occupied_positions.contains(&(nx, ny))
            && !is_base_cell(nx, ny, base_pos)
        {
            state.robots[robot_id].position = (nx, ny);
            return true;
        }
    }
    false
}

/// Deposits all resources the robot is carrying at the base and resets it to `Exploring`.
fn deposit_resources(state: &mut SimulationState, robot_id: usize) {
    state.deposited_crystals += state.robots[robot_id].carried_crystals;
    state.deposited_energy += state.robots[robot_id].carried_energy;
    state.robots[robot_id].carried_crystals = 0;
    state.robots[robot_id].carried_energy = 0;
    state.robots[robot_id].state = RobotState::Exploring;
}

/// Ticks a Scout robot: moves it and records any newly discovered resources at its position.
fn tick_scout<R: Rng>(
    state: &mut SimulationState,
    robot_id: usize,
    rng: &mut R,
    occupied_positions: &HashSet<(u16, u16)>,
) {
    let map = Arc::clone(&state.map);
    let width = state.width;
    let height = state.height;
    let base_pos = state.base_pos;

    {
        let robot = &mut state.robots[robot_id];
        move_scout(robot, &map, width, height, base_pos, rng, occupied_positions);
    }

    let (rx, ry) = state.robots[robot_id].position;

    // CORRECTION : Un scout ne peut détecter que si la ressource a réellement été générée
    if let Some(&qty) = state.resource_quantities.get(&(rx, ry)) {
        if qty > 0 {
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
    }
}

/// Ticks a Collector in `MovingToResource` state:
/// advances along its path, reroutes on collisions, and transitions to `Collecting`
/// or `Exploring` once the path is exhausted.
fn tick_collector_moving_to_resource(
    state: &mut SimulationState,
    robot_id: usize,
    occupied_positions: &HashSet<(u16, u16)>,
) {
    let next = state.robots[robot_id].path.pop_front();
    if let Some(next) = next {
        if occupied_positions.contains(&next) {
            let target = state.robots[robot_id].path.back().copied().unwrap_or(next);
            if !reroute_path(state, robot_id, target) {
                state.robots[robot_id].path.clear();
                state.robots[robot_id].state = RobotState::Exploring;
            }
        } else {
            state.robots[robot_id].position = next;
        }
    }

    if state.robots[robot_id].path.is_empty()
        && state.robots[robot_id].state == RobotState::MovingToResource
    {
        let pos = state.robots[robot_id].position;
        let qty = state.resource_quantities.get(&pos).copied().unwrap_or(0);
        state.robots[robot_id].state = if qty > 0 {
            RobotState::Collecting
        } else {
            RobotState::Exploring
        };
    }
}

/// Ticks a Collector in `ReturningToBase` state:
/// advances toward the base, reroutes on collisions (with sideways fallback),
/// and deposits resources upon arrival.
fn tick_collector_returning_to_base(
    state: &mut SimulationState,
    robot_id: usize,
    occupied_positions: &HashSet<(u16, u16)>,
) {
    let base_pos = state.base_pos;
    let next = state.robots[robot_id].path.pop_front();

    if let Some(next) = next {
        if occupied_positions.contains(&next) && next != base_pos {
            if !reroute_path(state, robot_id, base_pos) {
                // BFS failed: try a sideways step, otherwise wait in place
                if !step_sideways(state, robot_id, occupied_positions) {
                    state.robots[robot_id].path.push_front(next);
                }
            }
        } else {
            state.robots[robot_id].position = next;
            if next == base_pos {
                deposit_resources(state, robot_id);
            }
        }
    } else {
        state.robots[robot_id].state = RobotState::Exploring;
    }
}

/// Ticks a Collector in `Collecting` state:
/// extracts one unit of resource per tick, marks depleted tiles,
/// and triggers a return to base when the robot is full or the resource runs out.
fn tick_collector_collecting(state: &mut SimulationState, robot_id: usize) {
    let pos = state.robots[robot_id].position;
    let map = Arc::clone(&state.map);
    let base_pos = state.base_pos;
    let width = state.width;
    let height = state.height;
    let mut resource_depleted = false;

    if let Some(qty) = state.resource_quantities.get_mut(&pos) {
        if *qty > 0 {
            *qty -= 1;
            if is_crystal(&map, pos.0, pos.1) {
                state.robots[robot_id].carried_crystals += 1;
                if *qty == 0 {
                    resource_depleted = true;
                    state.collected_crystals.insert(pos);
                }
            } else if is_energy(&map, pos.0, pos.1) {
                state.robots[robot_id].carried_energy += 1;
                if *qty == 0 {
                    resource_depleted = true;
                    state.collected_energy.insert(pos);
                }
            }
        } else {
            resource_depleted = true;
        }
    }

    let total_carried =
        state.robots[robot_id].carried_crystals + state.robots[robot_id].carried_energy;
    if total_carried >= 25 || resource_depleted {
        let path = bfs(
            &map,
            &state.collected_crystals,
            &state.collected_energy,
            pos,
            base_pos,
            width,
            height,
            base_pos,
        );
        let robot = &mut state.robots[robot_id];
        robot.path = path;
        robot.state = RobotState::ReturningToBase;
    }
}

/// Ticks a Collector in any other state (Idle / Exploring):
/// moves it like a Scout and immediately switches to `Collecting`
/// if it lands on a resource tile.
fn tick_collector_exploring<R: Rng>(
    state: &mut SimulationState,
    robot_id: usize,
    rng: &mut R,
    occupied_positions: &HashSet<(u16, u16)>,
) {
    let map = Arc::clone(&state.map);
    let width = state.width;
    let height = state.height;
    let base_pos = state.base_pos;

    {
        let robot = &mut state.robots[robot_id];
        robot.state = RobotState::Exploring;
        move_scout(robot, &map, width, height, base_pos, rng, occupied_positions);
    }

    let pos = state.robots[robot_id].position;
    if let Some(&qty) = state.resource_quantities.get(&pos) {
        if qty > 0 && (is_crystal(&map, pos.0, pos.1) || is_energy(&map, pos.0, pos.1)) {
            let robot = &mut state.robots[robot_id];
            robot.state = RobotState::Collecting;
            robot.path.clear();
        }
    }
}

/// Dispatches one simulation tick for the given robot to the appropriate handler
/// based on its type and current state.
fn tick_robot<R: Rng>(state: &mut SimulationState, robot_id: usize, rng: &mut R) {
    let robot_type = state.robots[robot_id].robot_type;

    let occupied_positions: HashSet<(u16, u16)> = state.robots.iter()
        .enumerate()
        .filter(|(id, _)| *id != robot_id)
        .map(|(_, r)| r.position)
        .collect();

    match robot_type {
        RobotType::Scout => tick_scout(state, robot_id, rng, &occupied_positions),
        RobotType::Collector => {
            let current_state = state.robots[robot_id].state;
            match current_state {
                RobotState::MovingToResource => {
                    tick_collector_moving_to_resource(state, robot_id, &occupied_positions)
                }
                RobotState::ReturningToBase => {
                    tick_collector_returning_to_base(state, robot_id, &occupied_positions)
                }
                RobotState::Collecting => tick_collector_collecting(state, robot_id),
                _ => tick_collector_exploring(state, robot_id, rng, &occupied_positions),
            }
        }
    }
}

fn assign_collectors(state: &mut SimulationState) {
    let mut already_targeted: HashSet<(u16, u16)> = HashSet::new();

    for r in &state.robots {
        if r.robot_type == RobotType::Collector {
            if !r.path.is_empty() {
                if let Some(target) = r.path.back() {
                    already_targeted.insert(*target);
                }
            } else if r.state == RobotState::Collecting {
                already_targeted.insert(r.position);
            }
        }
    }

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
            || robot.state == RobotState::MovingToResource
            || robot.state == RobotState::Collecting
            || robot.state == RobotState::ReturningToBase
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
                robot.state = RobotState::MovingToResource;
                all_targets.remove(idx);
                already_targeted.insert(target);
            }
        }
    }
}

fn move_scout<R: Rng>(
    robot: &mut Robot,
    map: &[Vec<f64>],
    width: usize,
    height: usize,
    base_pos: (u16, u16),
    rng: &mut R,
    occupied_positions: &HashSet<(u16, u16)>,
) {
    let directions = [(0i16, -1i16), (0, 1), (-1, 0), (1, 0)];

    if rng.gen_bool(0.05) {
        robot.preferred_dir = directions[rng.gen_range(0..4)];
    }

    let mut alternatives: Vec<(i16, i16)> = directions
        .iter()
        .filter(|&&d| d != robot.preferred_dir)
        .copied()
        .collect();
    alternatives.shuffle(rng);

    let mut candidates = vec![robot.preferred_dir];
    candidates.extend(alternatives);

    for (dx, dy) in candidates {
        let nx = (robot.position.0 as i16 + dx).clamp(1, width as i16 - 2) as u16;
        let ny = (robot.position.1 as i16 + dy).clamp(1, height as i16 - 2) as u16;

        let actuellement_dans_la_base = is_base_cell(robot.position.0, robot.position.1, base_pos);

        let destination_valide = (!is_base_cell(nx, ny, base_pos) || actuellement_dans_la_base)
            && !occupied_positions.contains(&(nx, ny))
            && !is_obstacle(map, nx, ny);

        if destination_valide {
            robot.position = (nx, ny);
            robot.preferred_dir = (dx, dy);
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    fn setup_test_map() -> Vec<Vec<f64>> {
        vec![vec![0.0; 10]; 10]
    }

    #[test]
    fn test_safe_spawn_zone() {
        let map = App::generate_map(40, 40);
        for y in 5..=35 {
            for x in 5..=35 {
                let dx = x as i32 - 20;
                let dy = y as i32 - 20;
                if dx.abs().max(dy.abs()) <= 3 {
                    assert_eq!(map[y][x], 0.0, "Obstacle or resource in safe zone at ({}, {})", x, y);
                }
            }
        }
    }

    #[test]
    fn test_scout_movement() {
        let map = setup_test_map();
        let base_pos = (5, 5);
        let mut robot = Robot::new((2, 2), RobotType::Scout, (1, 0));
        let mut rng = StdRng::seed_from_u64(1234);
        let occupied = HashSet::new();

        move_scout(&mut robot, &map, 10, 10, base_pos, &mut rng, &occupied);

        let (x, y) = robot.position;
        let dx = (x as i16 - 2).abs();
        let dy = (y as i16 - 2).abs();
        assert_eq!(dx + dy, 1, "Scout must move exactly one tile orthogonally");
    }

    #[test]
    fn test_collector_collecting_state() {
        let mut map = setup_test_map();
        map[2][3] = 0.2; // crystal tile
        let base_pos = (5, 5);

        let mut robot = Robot::new((3, 2), RobotType::Collector, (0, 0));
        robot.state = RobotState::Collecting;

        let robots = vec![robot];
        let mut state = SimulationState::new_test(map, base_pos, robots);
        state.resource_quantities.insert((3, 2), 2);

        let mut rng = StdRng::seed_from_u64(1234);

        state.tick(&mut rng);
        assert_eq!(state.robots[0].carried_crystals, 1);
        assert_eq!(*state.resource_quantities.get(&(3, 2)).unwrap(), 1);
        assert_eq!(state.robots[0].state, RobotState::Collecting);
        assert!(!state.collected_crystals.contains(&(3, 2)));

        state.tick(&mut rng);
        assert_eq!(state.robots[0].carried_crystals, 2);
        assert_eq!(*state.resource_quantities.get(&(3, 2)).unwrap(), 0);
        assert_eq!(state.robots[0].state, RobotState::ReturningToBase);
        assert!(state.collected_crystals.contains(&(3, 2)));
    }

    #[test]
    fn test_collector_deposit_at_base() {
        let map = setup_test_map();
        let base_pos = (5, 5);

        let mut robot = Robot::new((5, 5), RobotType::Collector, (0, 0));
        robot.carried_crystals = 3;
        robot.carried_energy = 2;
        robot.state = RobotState::ReturningToBase;
        robot.path = VecDeque::from(vec![(5, 5)]);

        let robots = vec![robot];
        let mut state = SimulationState::new_test(map, base_pos, robots);

        let mut rng = StdRng::seed_from_u64(1234);
        state.tick(&mut rng);

        assert_eq!(state.deposited_crystals, 3);
        assert_eq!(state.deposited_energy, 2);
        assert_eq!(state.robots[0].carried_crystals, 0);
        assert_eq!(state.robots[0].carried_energy, 0);
        assert_eq!(state.robots[0].state, RobotState::Exploring);
    }

    #[test]
    fn test_collector_returning_does_not_collect() {
        // A collector in ReturningToBase should not pick up resources along the way.
        let mut map = setup_test_map();
        map[5][4] = 0.2; // crystal at (4,5)

        let base_pos = (5, 5);
        let mut robot = Robot::new((3, 5), RobotType::Collector, (0, 0));
        robot.carried_crystals = 1;
        robot.state = RobotState::ReturningToBase;
        robot.path = VecDeque::from(vec![(4, 5), (5, 5)]);

        let robots = vec![robot];
        let mut state = SimulationState::new_test(map, base_pos, robots);
        state.resource_quantities.insert((4, 5), 50);
        let mut rng = StdRng::seed_from_u64(1234);

        state.tick(&mut rng);

        // Robot moves to (4,5) but doesn't collect the crystal there
        assert_eq!(state.robots[0].position, (4, 5));
        assert_eq!(state.robots[0].carried_crystals, 1);
        assert!(!state.collected_crystals.contains(&(4, 5)));
    }

    #[test]
    fn test_scout_avoids_occupied_positions() {
        let map = setup_test_map();
        let base_pos = (5, 5);
        let mut robot = Robot::new((2, 2), RobotType::Scout, (1, 0));
        let mut rng = StdRng::seed_from_u64(1234);

        // Block all directions except one
        let mut occupied = HashSet::new();
        occupied.insert((3, 2)); // right
        occupied.insert((1, 2)); // left
        occupied.insert((2, 1)); // up

        move_scout(&mut robot, &map, 10, 10, base_pos, &mut rng, &occupied);

        // Should move down to (2, 3) — the only unoccupied direction
        assert_eq!(robot.position, (2, 3));
    }
}

