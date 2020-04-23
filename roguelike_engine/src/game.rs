use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use rand::prelude::*;

use slotmap::dense::*;

use serde::{Serialize, Deserialize};

use sdl2::keyboard::Keycode;

use roguelike_core::constants::*;
use roguelike_core::types::*;
use roguelike_core::config::*;
use roguelike_core::ai::*;
use roguelike_core::map::*;
use roguelike_core::messaging::{Msg, MsgLog};
use roguelike_core::movement::{Direction, Action};
use roguelike_core::utils::{clampf, lerp};

use crate::actions;
use crate::actions::{InputAction, KeyDirection};
use crate::generation::*;
use crate::display::*;
use crate::read_map::read_map_xp;
use crate::console::Console;


#[derive(Copy, Clone, PartialEq, Serialize, Deserialize)]
pub enum GameResult {
    Continue,
    Stop,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct GameSettings {
    pub turn_count: usize,
    pub god_mode: bool,
    pub map_type: MapGenType,
    pub exiting: bool,
    pub state: GameState,
    pub draw_throw_overlay: bool,
    pub overlay: bool,
    pub console: bool,
    pub time: f32,
    pub render_map: bool,
}

impl GameSettings {
    pub fn new(turn_count: usize,
               god_mode: bool) -> GameSettings {
        return GameSettings {
            turn_count,
            god_mode,
            map_type: MapGenType::Island,
            exiting: false,
            state: GameState::Playing,
            draw_throw_overlay: false,
            overlay: false,
            console: false,
            time: 0.0,
            render_map: true,
        };
    }
}

pub struct Game {
    pub config: Config,

    pub input_action: InputAction,

    pub key_input: Vec<(KeyDirection, Keycode)>,

    pub mouse_state: MouseState,

    pub display_state: DisplayState,

    pub data: GameData,

    pub settings: GameSettings,

    pub msg_log: MsgLog,

    pub console: Console,
}

impl Game {
    pub fn new(args: &Vec<String>,
               config: Config,
               mut display_state: DisplayState) -> Result<Game, String> {
        // Create seed for random number generator, either from
        // user input or randomly
        let seed: u64;
        if args.len() > 1 {
            let mut hasher = DefaultHasher::new();
            args[1].hash(&mut hasher);
            seed = hasher.finish();
        } else {
            seed = rand::thread_rng().gen();
        }
        println!("Seed: {} (0x{:X})", seed, seed);

        let mut objects = DenseSlotMap::with_capacity(INITIAL_OBJECT_CAPACITY);
        let mut rng: SmallRng = SeedableRng::seed_from_u64(seed);

        let map;
        let player_position: (i32, i32);
        match config.map_load {
            MapLoadConfig::FromFile => {
                let (new_objects, new_map, mut position) = read_map_xp(&config, &mut display_state, "resources/map.xp");
                objects.clear();
                for object in new_objects.values() {
                    objects.insert(object.clone());
                }
                map = new_map;
                if position == (0, 0) {
                    position = (map.width() / 2, map.height() / 2);
                }
                player_position = position;

                objects.insert(make_mouse(&config, &mut display_state));
            }

            MapLoadConfig::Random => {
                let (data, position) =
                    make_map(&MapGenType::Island, &mut objects, &config, &mut display_state, &mut rng);
                // TODO consider using objects as well here on regen?
                map = data.map;
                player_position = position.to_tuple();
            }

            MapLoadConfig::TestWall => {
                let (new_map, position) = make_wall_test_map(&mut objects, &config, &mut display_state);
                map = new_map;
                player_position = position.to_tuple();
            }

            MapLoadConfig::TestPlayer => {
                let (new_map, position) = make_player_test_map(&mut objects, &config, &mut display_state);
                map = new_map;
                player_position = position.to_tuple();
            }

            MapLoadConfig::TestCorner => {
                let (new_map, position) = make_corner_test_map(&mut objects, &config, &mut display_state);
                map = new_map;
                player_position = position.to_tuple();
                objects.insert(make_mouse(&config, &mut display_state));
            }

            MapLoadConfig::Animations => {
                let (new_map, position) = make_animations_map(&mut objects, &config, &mut display_state);
                map = new_map;
                player_position = position.to_tuple();
            }
        }

        let mut data = GameData::new(map, objects);

        let player_id = data.objects.insert(make_player(&config, &mut display_state));
        data.objects[player_id].x = player_position.0;
        data.objects[player_id].y = player_position.1;

        let stone_id = data.objects.insert(make_stone(&config, Pos::new(-1, -1)));
        data.objects[player_id].inventory.push_back(stone_id);

        let state = Game {
            config,
            input_action: InputAction::None,
            data,
            display_state,
            settings: GameSettings::new(0, false),
            mouse_state: Default::default(),
            msg_log: MsgLog::new(),
            console: Console::new(),
            key_input: Vec::new(),
        };

        return Ok(state);
    }

    pub fn step_game(&mut self, dt: f32) -> GameResult {
        self.settings.time += dt;

        match self.settings.state {
            GameState::Playing => {
                return self.step_playing();
            }

            GameState::Win => {
                return self.step_win();
            }

            GameState::Lose => {
                return self.step_lose();
            }

            GameState::Inventory => {
                return self.step_inventory();
            }

            GameState::Throwing => {
                return self.step_throwing();
            }

            GameState::Console => {
                return self.step_console();
            }
        }
    }

    fn step_win(&mut self) -> GameResult {

        if matches!(self.input_action, InputAction::Exit) {
            return GameResult::Stop;
        }

        let player_id = self.data.find_player().unwrap();

        let (new_objects, new_map, _) =
            read_map_xp(&self.config, &mut self.display_state, "resources/map.xp");

        self.data.map = new_map;
        self.data.objects[player_id].inventory.clear();
        let player = self.data.objects[player_id].clone();
        self.data.objects.clear();
        self.data.objects.insert(player);
        for key in new_objects.keys() {
            self.data.objects.insert(new_objects[key].clone());
        }

        self.settings.state = GameState::Playing;

        // NOTE Exit game on win for now
        return GameResult::Stop;
    }

    fn step_lose(&mut self) -> GameResult {
        if self.input_action == InputAction::Exit {
            return GameResult::Stop;
        }

        return GameResult::Continue;
    }

    fn step_inventory(&mut self) -> GameResult {
        let input = self.input_action;
        self.input_action = InputAction::None;

        actions::handle_input_inventory(input, &mut self.data, &mut self.settings, &mut self.msg_log);

        if self.settings.exiting {
            return GameResult::Stop;
        }

        return GameResult::Continue;
    }

    fn step_throwing(&mut self) -> GameResult {
        let input = self.input_action;
        self.input_action = InputAction::None;

        self.settings.draw_throw_overlay = true;

        let player_action =
            actions::handle_input_throwing(input,
                                           &mut self.data,
                                           &mut self.settings,
                                           &mut self.msg_log);

        if player_action != Action::NoAction {
            step_logic(player_action,
                       &mut self.data,
                       &mut self.settings,
                       &self.config,
                       &mut self.msg_log);
        }

        if self.settings.exiting {
            return GameResult::Stop;
        }

        return GameResult::Continue;
    }

    fn step_console(&mut self) -> GameResult {
        let input = self.input_action;
        self.input_action = InputAction::None;

        let time_since_open = self.settings.time - self.console.time_at_open;
        let lerp_amount = clampf(time_since_open / self.config.console_speed, 0.0, 1.0);
        self.console.height = lerp(self.console.height as f32,
                                   self.config.console_max_height as f32,
                                   lerp_amount) as u32;
        if (self.console.height as i32 - self.config.console_max_height as i32).abs() < 2 {
            self.console.height = self.config.console_max_height;
        }

        if self.key_input.len() > 0 {
            actions::handle_input_console(input,
                                          &mut self.key_input,
                                          &mut self.console,
                                          &mut self.data,
                                          &mut self.display_state,
                                          &mut self.settings,
                                          &self.config,
                                          &mut self.msg_log);
        }

        return GameResult::Continue;
    }

    fn step_playing(&mut self) -> GameResult {
        let player_action =
            actions::handle_input(self);

        if player_action != Action::NoAction {
            step_logic(player_action,
                       &mut self.data,
                       &mut self.settings,
                       &self.config,
                       &mut self.msg_log);

            if win_condition_met(&self.data) {
                self.settings.state = GameState::Win;
            }
            self.settings.turn_count += 1;
        }

        if self.settings.exiting {
            return GameResult::Stop;
        }

        self.input_action = InputAction::None;

        return GameResult::Continue;
    }
}

/// Check whether the exit condition for the game is met.
fn win_condition_met(data: &GameData) -> bool {
    // loop over objects in inventory, and check whether any
    // are the key object.
    //let has_key =
    //inventory.iter().any(|obj| obj.item.map_or(false, |item| item == Item::Goal));
    // TODO add back in with new inventory!
    let player_id = data.find_player().unwrap();

    let has_key = 
        data.objects[player_id].inventory.iter().any(|item_id| {
            data.objects[*item_id].item == Some(Item::Goal)
        });

    let player_pos = data.objects[player_id].pos();
    let on_exit_tile = data.map[player_pos].tile_type == TileType::Exit;

    let exit_condition = has_key && on_exit_tile;

    return exit_condition;
}

pub fn step_logic(player_action: Action,
                  data: &mut GameData, 
                  settings: &mut GameSettings,
                  config: &Config,
                  msg_log: &mut MsgLog) {
    let player_id = data.find_player().unwrap();

    let previous_player_position =
        data.objects[player_id].pos();

    actions::player_apply_action(player_action, data, msg_log);

    /* AI */
    if data.objects[player_id].alive {
        let mut ai_id = Vec::new();

        for key in data.objects.keys() {
            if data.objects[key].ai.is_some() &&
               data.objects[key].alive        &&
               data.objects[key].fighter.is_some() {
               ai_id.push(key);
           }
        }

        for key in ai_id {
            ai_take_turn(key, data, config, msg_log);

            // check if fighter needs to be removed
            if let Some(fighter) = data.objects[key].fighter {
                if fighter.hp <= 0 {
                    data.objects[key].alive = false;
                    data.objects[key].blocks = false;
                    data.objects[key].chr = '%';
                    //data.objects[key].color = config.color_red;
                    data.objects[key].fighter = None;
                }
            }
        }
    }

    /* Traps */
    let mut traps = Vec::new();
    for key in data.objects.keys() {
        for other in data.objects.keys() {
            if data.objects[key].trap.is_some() && // key is a trap
               data.objects[other].alive && // entity is alive
               data.objects[other].fighter.is_some() && // entity is a fighter
               data.objects[key].pos() == data.objects[other].pos() {
                traps.push((key, other));
            }
        }
    }

    for (trap, entity) in traps.iter() {
        match data.objects[*trap].trap.unwrap() {
            Trap::Spikes => {
                data.objects[*entity].take_damage(SPIKE_DAMAGE);

                msg_log.log(Msg::SpikeTrapTriggered(*trap, *entity));

                if data.objects[*entity].fighter.unwrap().hp <= 0 {
                    data.objects[*entity].alive = false;
                    data.objects[*entity].blocks = false;

                    msg_log.log(Msg::Killed(*trap, *entity, SPIKE_DAMAGE));
                }

                data.objects[*trap].needs_removal = true;
            }

            Trap::Sound => {
                msg_log.log(Msg::SoundTrapTriggered(*trap, *entity));
            }
        }
    }

    // check if player lost all hp
    if let Some(fighter) = data.objects[player_id].fighter {
        if fighter.hp <= 0 {
            // modify player
            {
                let player = &mut data.objects[player_id];
                player.alive = false;
                player.color = config.color_red;
                player.fighter = None;
            }

            if settings.state == GameState::Playing {
                settings.state = GameState::Lose;
            }
        }
    }

    let mut to_remove = Vec::new();
    for (entity_key, entity) in data.objects.iter_mut() {
        if let Some(ref mut count) = entity.count_down {
            if *count == 0 {
                to_remove.push(entity_key);
            } else {
                *count -= 1;
            }
        }

        if entity.needs_removal {
            to_remove.push(entity_key);
        }
    }
    for key in to_remove {
        data.objects.remove(key);
    }

    /* Recompute FOV */
    let player_pos = data.objects[player_id].pos();
    if previous_player_position != player_pos {
        data.map.compute_fov(player_pos, config.fov_radius_player);
    }
}

