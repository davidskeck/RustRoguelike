use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use rand::prelude::*;

use slotmap::dense::*;

use roguelike_core::constants::*;
use roguelike_core::types::*;
use roguelike_core::config::*;
use roguelike_core::ai::*;
use roguelike_core::map::*;
use roguelike_core::messaging::{Msg, MsgLog};

use crate::actions;
use crate::generation::*;
use crate::display::*;
use crate::input::*;
use crate::read_map::read_map_xp;


#[derive(Copy, Clone, PartialEq)]
pub enum GameResult {
    Continue,
    Stop,
}

pub struct GameSettings {
    pub turn_count: usize,
    pub god_mode: bool,
    pub map_type: MapGenType,
    pub exiting: bool,
    pub state: GameState,
}

impl GameSettings {
    pub fn new(turn_count: usize,
               god_mode: bool) -> GameSettings {
        GameSettings {
            turn_count,
            god_mode,
            map_type: MapGenType::Island,
            exiting: false,
            state: GameState::Playing,
        }
    }
}

pub struct Game<'a> {
    pub config: Config,

    pub input_action: InputAction,

    pub mouse_state: MouseState,

    pub display_state: DisplayState<'a>,

    pub data: GameData,

    pub settings: GameSettings,

    pub msg_log: MsgLog,
}

impl<'a> Game<'a> {
    pub fn new(args: &Vec<String>,
               config: Config,
               display_state: DisplayState<'a>) -> Result<Game<'a>, String> {
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

        let mut map;
        let player_position;
        match config.map_load {
            MapLoadConfig::FromFile => {
                let (new_objects, new_map, mut position) = read_map_xp(&config, &display_state, "resources/map.xp");
                objects.clear();
                for object in new_objects.values() {
                    objects.insert(object.clone());
                }
                map = new_map;
                if position == (0, 0) {
                    position = (map.width() / 2, map.height() / 2);
                }
                player_position = position;

                objects.insert(make_goal(&config, Pos::new(player_position.0 - 1, player_position.1)));
                objects.insert(make_mouse(&config, &display_state));
                objects.insert(make_spikes(&config, Pos::new(player_position.0, player_position.1 - 2), &display_state));

                let exit_position = (player_position.0 + 1, player_position.1 - 1);
                map[exit_position].tile_type = TileType::Exit;
                map[exit_position].chr = Some(MAP_ORB as char);
            }

            MapLoadConfig::Random => {
                let (game_data, position) =
                    make_map(&MapGenType::Island, &mut objects, &config, &display_state, &mut rng);
                // TODO consider using objects as well here on regen?
                map = game_data.map;
                player_position = position.to_tuple();
            }

            MapLoadConfig::TestWall => {
                let (new_map, position) = make_wall_test_map(&mut objects, &config, &display_state);
                map = new_map;
                player_position = position.to_tuple();
            }

            MapLoadConfig::TestPlayer => {
                let (new_map, position) = make_player_test_map(&mut objects, &config, &display_state);
                map = new_map;
                player_position = position.to_tuple();
            }

            MapLoadConfig::TestCorner => {
                let (new_map, position) = make_corner_test_map(&mut objects, &config, &display_state);
                map = new_map;
                player_position = position.to_tuple();
                objects.insert(make_mouse(&config, &display_state));
            }
        }

        let mut data = GameData::new(map, objects);

        let player_handle = data.objects.insert(make_player(&config, &display_state));
        data.objects[player_handle].x = player_position.0;
        data.objects[player_handle].y = player_position.1;

        let stone_handle = data.objects.insert(make_stone(&config, Pos::new(-1, -1)));
        data.objects[player_handle].inventory.push(stone_handle);

        let state = Game {
            config,
            input_action: InputAction::None,
            data,
            display_state,
            settings: GameSettings::new(0, false),
            mouse_state: Default::default(),
            msg_log: MsgLog::new(),
        };

        Ok(state)
    }

    pub fn step_game(&mut self) -> GameResult {

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
        }
    }

    fn step_win(&mut self) -> GameResult {

        match self.input_action {
            InputAction::Exit => {
                return GameResult::Stop;
            }

            _ => {},
        }

        let player_handle = self.data.find_player().unwrap();

        let (new_objects, new_map, _) = read_map_xp(&self.config, &self.display_state, "resources/map.xp");
        self.data.map = new_map;
        self.data.objects[player_handle].inventory.clear();
        let player = self.data.objects[player_handle].clone();
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

    fn step_playing(&mut self) -> GameResult {
        let player_handle = self.data.find_player().unwrap();

        // clear input action
        let input = self.input_action;
        self.input_action = InputAction::None;

        let player_action =
            actions::handle_input(input,
                                  &mut self.data,
                                  &mut self.settings,
                                  &mut self.display_state,
                                  &self.config);

        if player_action != Action::NoAction {
            step_logic(player_action,
                       &mut self.data,
                       &mut self.settings,
                       &self.config,
                       &mut self.msg_log);

            if win_condition_met(&self.data) {
                self.settings.state = GameState::Win;
            }
        } else {
            self.settings.turn_count += 1;
        }

        /* Check Exit Conditions */
        if self.settings.exiting {
            return GameResult::Stop;
        }

        return GameResult::Continue;
    }
}

/// Check whether the exit condition for the game is met.
fn win_condition_met(data: &GameData) -> bool {
    // loop over objects in inventory, and check whether any
    // are the goal object.
    //let has_goal =
    //inventory.iter().any(|obj| obj.item.map_or(false, |item| item == Item::Goal));
    // TODO add back in with new inventory!
    let player_handle = data.find_player().unwrap();

    let has_goal = 
        data.objects[player_handle].inventory.iter().any(|item_handle| {
            data.objects[*item_handle].item == Some(Item::Goal)
        });

    let player_pos = data.objects[player_handle].pos();
    let on_exit_tile = data.map[player_pos].tile_type == TileType::Exit;

    let exit_condition = has_goal && on_exit_tile;

    return exit_condition;
}

pub fn step_logic(player_action: Action,
                  game_data: &mut GameData, 
                  settings: &mut GameSettings,
                  config: &Config,
                  msg_log: &mut MsgLog) {
    let player_handle = game_data.find_player().unwrap();

    let previous_player_position =
        game_data.objects[player_handle].pos();

    actions::player_apply_action(player_action, game_data, msg_log);

    /* AI */
    if game_data.objects[player_handle].alive {
        let mut ai_handles = Vec::new();

        for key in game_data.objects.keys() {
            if game_data.objects[key].ai.is_some() &&
               game_data.objects[key].fighter.is_some() {

               ai_handles.push(key);
           }
        }

        for key in ai_handles {
            ai_take_turn(key, game_data, msg_log);

            // check if fighter needs to be removed
            if let Some(fighter) = game_data.objects[key].fighter {
                if fighter.hp <= 0 {
                    game_data.objects[key].alive = false;
                    game_data.objects[key].chr = '%';
                    game_data.objects[key].color = config.color_red;
                    game_data.objects[key].fighter = None;

                    game_data.objects.remove(key);
                }
            }
        }
    }

    /* Traps */
    let mut traps = Vec::new();
    for key in game_data.objects.keys() {
        for other in game_data.objects.keys() {
            if game_data.objects[key].trap.is_some() && // key is a trap
               game_data.objects[other].alive && // entity is alive
               game_data.objects[other].fighter.is_some() && // entity is a fighter
               game_data.objects[key].pos() == game_data.objects[other].pos() {
                traps.push((key, other));
            }
        }
    }

    for (trap, entity) in traps.iter() {
        match game_data.objects[*trap].trap.unwrap() {
            Trap::Spikes => {
                game_data.objects[*entity].take_damage(SPIKE_DAMAGE);

                msg_log.log(Msg::SpikeTrapTriggered(*trap, *entity));
            }
        }
    }

    // TODO move enemy health checks here for trap damage

    // check if player lost all hp
    if let Some(fighter) = game_data.objects[player_handle].fighter {
        if fighter.hp <= 0 {
            // modify player
            {
                let player = &mut game_data.objects[player_handle];
                player.alive = false;
                player.color = config.color_red;
                player.fighter = None;
            }

            if settings.state == GameState::Playing {
                settings.state = GameState::Lose;
            }
        }
    }

    /* Recompute FOV */
    let player_pos = game_data.objects[player_handle].pos();
    if previous_player_position != player_pos {
        game_data.map.compute_fov(player_pos, FOV_RADIUS);
    }
}
