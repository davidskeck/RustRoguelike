use rand::prelude::*;

use serde::{Serialize, Deserialize};

use sdl2::keyboard::Keycode;

use roguelike_core::constants::*;
use roguelike_core::types::*;
use roguelike_core::config::*;
use roguelike_core::ai::*;
use roguelike_core::map::*;
use roguelike_core::messaging::{Msg, MsgLog};
use roguelike_core::movement::{Action, Reach};
use roguelike_core::utils::{move_towards, distance, add_pos, signedness, sub_pos};

use crate::actions;
use crate::actions::{InputAction, KeyDirection};
use crate::generation::*;
use crate::make_map::read_map_xp;
use crate::resolve::resolve_messages;


#[derive(Copy, Clone, PartialEq, Serialize, Deserialize)]
pub enum GameResult {
    Continue,
    Stop,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub enum SelectionAction {
    Throw,
    Hammer,
}

impl SelectionAction {
    pub fn action_from_pos(&self, pos: Pos, data: &GameData) -> Action {
        let action: Action;

        match self {
            Throw => {
                let player_id = data.find_player().unwrap();
                action = Action::ThrowItem(pos, player_id);
            }

            Hammer => {
                action = Action::UseItem(pos);
            }
        }

        return action;
    }
}


#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub enum SelectionType {
    WithinReach(Reach),
    WithinRadius(usize),
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Selection {
    typ: SelectionType,
    action: SelectionAction,
    // TODO consider adding:
    // SelectionFilter enum with Entity/Wall/Empty/Any
    // position to selection will have to check available positions and find one that matches
    // the filter
}

impl Default for Selection {
    fn default() -> Selection {
        return Selection::new(SelectionType::WithinRadius(0), SelectionAction::Throw);
    }
}

impl Selection {
    pub fn new(typ: SelectionType, action: SelectionAction) -> Self {
        return Selection {
            typ,
            action,
        };
    }

    pub fn selected_pos(&self, pos: Pos, selected: Pos, data: &GameData) -> Option<Pos> {
        let mut maybe_selected_pos: Option<Pos> = None;

        match self.typ {
            SelectionType::WithinReach(reach) => {
                let selected_pos = reach.closest_to(pos, selected);

                maybe_selected_pos = Some(selected_pos);
            }

            SelectionType::WithinRadius(radius) => {
                let selected_pos: Pos;
                if distance(selected, pos) as usize <= radius {
                    selected_pos = selected;
                } else {
                    selected_pos = move_towards(pos, selected, radius);
                }

                maybe_selected_pos = Some(selected_pos);
            }
        }

        return maybe_selected_pos;
    }

    pub fn select(&self, pos: Pos, selected: Pos, data: &GameData) -> Option<Action> {
        let maybe_selected_pos: Option<Pos> = self.selected_pos(pos, selected, data);

        if let Some(selected_pos) = maybe_selected_pos {
            return Some(self.action.action_from_pos(selected_pos, data));
        } else {
            return None;
        }
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct GameSettings {
    pub turn_count: usize,
    pub god_mode: bool,
    pub map_type: MapGenType,
    pub exiting: bool,
    pub state: GameState,
    // TODO remove these two- subsumed by selection overlay
    pub draw_throw_overlay: bool,
    pub draw_interact_overlay: bool,
    pub draw_selection_overlay: bool,
    pub overlay: bool,
    pub console: bool,
    pub time: f32,
    pub render_map: bool,
    pub selection: Selection,
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
            draw_interact_overlay: false,
            draw_selection_overlay: false,
            overlay: false,
            console: false,
            time: 0.0,
            render_map: true,
            selection: Selection::default(),
        };
    }
}

pub struct Game {
    pub config: Config,
    pub input_action: InputAction,
    pub key_input: Vec<(KeyDirection, Keycode)>,
    pub mouse_state: MouseState,
    pub data: GameData,
    pub settings: GameSettings,
    pub msg_log: MsgLog,
    pub rng: SmallRng,
}

impl Game {
    pub fn new(seed: u64, config: Config) -> Result<Game, String> {
        let entities = Entities::new();
        let rng: SmallRng = SeedableRng::seed_from_u64(seed);

        let mut msg_log = MsgLog::new();

        let map = Map::empty();

        let mut data = GameData::new(map, entities);

        let player_id = make_player(&mut data.entities, &config, &mut msg_log);
        data.entities.pos[&player_id] = Pos::new(-1, -1);

        let stone_id = make_stone(&mut data.entities, &config, Pos::new(-1, -1), &mut msg_log);
        data.entities.inventory[&player_id].push_back(stone_id);

        let state = Game {
            config,
            input_action: InputAction::None,
            data,
            settings: GameSettings::new(0, false),
            mouse_state: Default::default(),
            msg_log,
            key_input: Vec::new(),
            rng: rng,
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

            // TODO remove throwing and interact
            GameState::Throwing => {
                return self.step_throwing();
            }

            GameState::Interact => {
                return self.step_interact();
            }

            GameState::Selection => {
                return self.step_selection();
            }
        }
    }

    fn step_win(&mut self) -> GameResult {

        if matches!(self.input_action, InputAction::Exit) {
            return GameResult::Stop;
        }

        self.msg_log.log(Msg::ChangeLevel());

        self.data.entities.clear();
        let _player_pos =
            read_map_xp(&self.config, &mut self.data, &mut self.msg_log, "resources/map.xp");

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

    fn step_interact(&mut self) -> GameResult {
        let input = self.input_action;
        self.input_action = InputAction::None;

        self.settings.draw_interact_overlay = true;

        let player_action =
            actions::handle_input_interact(input,
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

    fn step_selection(&mut self) -> GameResult {
        let input = self.input_action;
        self.input_action = InputAction::None;

        // TODO make this a more generic selection overlay
        self.settings.draw_selection_overlay = true;

        // TODO implement selection handling
        let player_action =
            actions::handle_input_selection(input,
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

//    fn step_console(&mut self) -> GameResult {
//        let input = self.input_action;
//        self.input_action = InputAction::None;
//
//        let time_since_open = self.settings.time - self.console.time_at_open;
//        let lerp_amount = clampf(time_since_open / self.config.console_speed, 0.0, 1.0);
//        self.console.height = lerp(self.console.height as f32,
//                                   self.config.console_max_height as f32,
//                                   lerp_amount) as u32;
//        if (self.console.height as i32 - self.config.console_max_height as i32).abs() < 2 {
//            self.console.height = self.config.console_max_height;
//        }
//
//        if self.key_input.len() > 0 {
//            // TODO add console back in
//            //actions::handle_input_console(input,
//            //                              &mut self.key_input,
//            //                              &mut self.console,
//            //                              &mut self.data,
//            //                              &mut self.display_state,
//            //                              &mut self.settings,
//            //                              &self.config,
//            //                              &mut self.msg_log);
//        }
//
//        return GameResult::Continue;
//    }

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
    let player_id = data.find_player().unwrap();

    let has_key = 
        data.entities.inventory[&player_id].iter().any(|item_id| {
            data.entities.item.get(item_id) == Some(&Item::Goal)
        });

    let player_pos = data.entities.pos[&player_id];
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
        data.entities.pos[&player_id];

    data.entities.action[&player_id] = player_action;

    /* Actions */
    msg_log.log(Msg::Action(player_id, player_action));
    resolve_messages(data, msg_log, settings, config);

    if data.entities.alive[&player_id] {
        let mut ai_id: Vec<EntityId> = Vec::new();

        for key in data.entities.ids.iter() {
            if data.entities.ai.get(key).is_some() &&
               data.entities.alive[key]            &&
               data.entities.fighter.get(key).is_some() {
               ai_id.push(*key);
           }
        }

        for key in ai_id.iter() {
           data.entities.action[key] = ai_take_turn(*key, data, config, msg_log);
        }

        for key in ai_id {
            if let Some(action) = data.entities.action.get(&key).map(|v| *v) {
                msg_log.log(Msg::Action(key, action));
                resolve_messages(data, msg_log, settings, config);

                // check if fighter needs to be removed
                if let Some(fighter) = data.entities.fighter.get(&key) {
                    if fighter.hp <= 0 {
                        data.entities.alive[&key] = false;
                        data.entities.blocks[&key] = false;
                        data.entities.chr[&key] = '%';
                        data.entities.fighter.remove(&key);
                    }
                }
            }
        }
    }

    // TODO this shouldn't be necessary- it should be part of msg handling
    // check if player lost all hp
    if let Some(fighter) = data.entities.fighter.get(&player_id) {
        if fighter.hp <= 0 {
            // modify player
            {
                data.entities.alive[&player_id] = false;
                data.entities.color[&player_id] = config.color_red;
            }

            if settings.state == GameState::Playing {
                settings.state = GameState::Lose;
            }
        }
    }

    let mut to_remove: Vec<EntityId> = Vec::new();

    // perform count down
    for entity_id in data.entities.ids.iter() {
        if let Some(ref mut count) = data.entities.count_down.get_mut(entity_id) {
            if **count == 0 {
                to_remove.push(*entity_id);
            } else {
                **count -= 1;
            }
        }

        if data.entities.needs_removal[entity_id] &&
           data.entities.animation[entity_id].len() == 0 {
            to_remove.push(*entity_id);
        }
    }

    // remove objects waiting removal
    for key in to_remove {
        data.entities.remove(&key);
    }

    /* Recompute FOV */
    let player_pos = data.entities.pos[&player_id];
    if previous_player_position != player_pos {
        data.map.compute_fov(player_pos, config.fov_radius_player);
    }
}

