pub mod game;
pub mod display;
pub mod input;
pub mod plat;
pub mod read_map;
pub mod actions;
pub mod generation;
pub mod render;
mod throttler;

use std::time::Duration;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use sdl2::event::Event;
use sdl2::image::LoadTexture;
use sdl2::mouse::MouseButton;

use slotmap::dense::*;

use walkdir::WalkDir;

use roguelike_core::types::*;
use roguelike_core::map;
use roguelike_core::config::*;
use roguelike_core::messaging::Msg;
use roguelike_core::constants::*;
use roguelike_core::animation::{Effect, Animation};
use roguelike_core::movement::{MoveMode, Movement};

use crate::display::*;
use crate::render::*;
use crate::plat::*;
use crate::game::*;
use crate::input::*;
use crate::throttler::*;
use crate::read_map::read_map_xp;


pub fn run(args: &Vec<String>, config: Config) -> Result<(), String> {
    /* Create SDL Context */
    let sdl_context = sdl2::init()?;
    let video = sdl_context.video()?;
    let window = video.window("Rust Roguelike", SCREEN_WIDTH, SCREEN_HEIGHT)
        .position_centered().build().map_err(|e| e.to_string())?;

    let canvas = window.into_canvas()
        .accelerated().build().map_err(|e| e.to_string())?;
    let texture_creator = canvas.texture_creator();

    let mut event_pump = sdl_context.event_pump()?;

    /* Setup FPS Throttling */
    let fps_throttler = Throttler::new(Duration::from_millis(1000 / config.rate as u64));

    /* Load Textures */
    let font_image = texture_creator.load_texture("resources/rexpaint16x16.png")
        .map_err(|e| e.to_string())?;

    let player_idle = texture_creator.load_texture("animations/player/Player_Idle.png")
        .map_err(|e| e.to_string())?;

    let player_attack = texture_creator.load_texture("animations/player/player_attack.png")
        .map_err(|e| e.to_string())?;

    let player_wall_kick = texture_creator.load_texture("animations/player/player_wallkick.png")
        .map_err(|e| e.to_string())?;

    let gol_idle = texture_creator.load_texture("animations/monster1/Gol_Idle.png")
        .map_err(|e| e.to_string())?;

    let gol_die = texture_creator.load_texture("animations/monster1/Gol_Die.png")
        .map_err(|e| e.to_string())?;

    let elf_idle = texture_creator.load_texture("animations/monster3/Elf_Idle.png")
        .map_err(|e| e.to_string())?;

    let spikes_anim = texture_creator.load_texture("animations/traps/DamageTrap.png")
        .map_err(|e| e.to_string())?;

    let font_as_sprite = texture_creator.load_texture("resources/rexpaint16x16.png")
        .map_err(|e| e.to_string())?;

    let mcmuffin = texture_creator.load_texture("animations/traps/McMuffin.png")
        .map_err(|e| e.to_string())?;


    let mut sprites = DenseSlotMap::new();
    sprites.insert(SpriteSheet::new("player_wall_kick".to_string(), player_wall_kick, 1));
    sprites.insert(SpriteSheet::new("player_idle".to_string(),      player_idle,      1));
    sprites.insert(SpriteSheet::new("player_attack".to_string(),    player_attack,    1));
    sprites.insert(SpriteSheet::new("gol_idle".to_string(),         gol_idle,         1));
    sprites.insert(SpriteSheet::new("gol_die".to_string(),          gol_die,          1));
    sprites.insert(SpriteSheet::new("elf_idle".to_string(),         elf_idle,         1));
    sprites.insert(SpriteSheet::new("spikes".to_string(),           spikes_anim,      1));
    sprites.insert(SpriteSheet::new("font".to_string(),             font_as_sprite,   16));
    sprites.insert(SpriteSheet::new("goal".to_string(),             mcmuffin,         1));

    // load any animations in the autoload directory.
    for entry in WalkDir::new("animations/autoload/") {
        let entry = entry.unwrap();
        let path = entry.path();
        let file_name = entry.file_name().to_string_lossy().to_string();
        if let Ok(metadata) = entry.metadata() {
            println!("{:?} ends with png {}", path, file_name.ends_with("png"));
            if metadata.is_file() && file_name.ends_with("png") {
                let sprite =
                    texture_creator.load_texture(path).map_err(|e| e.to_string())?;

                println!("{}", file_name);
                sprites.insert(SpriteSheet::new(file_name, sprite, 1));
            }
        }
    }

    /* Create Display Structures */
    let screen_sections =
        Plan::vert("screen", 0.80, Plan::zone("map"),
                   Plan::split_horiz(0.3, Plan::zone("inventory"),
                                          Plan::split_horiz(0.5, Plan::zone("player"),
                                                                 Plan::zone("info"))));


    let display_state =
        DisplayState::new(screen_sections, font_image, sprites, canvas);

    let mut game = Game::new(args, config.clone(), display_state)?;

    /* Main Game Loop */
    let mut running = true;
    while running {
        /* Handle Events */
        for event in event_pump.poll_iter() {
            match event {
                Event::Quit {..}=> {
                    running = false;
                }

                Event::KeyUp {keycode, keymod, ..} => {
                    if let Some(keycode) = keycode {
                        game.input_action =
                            map_keycode_to_action(keycode, keymod, game.settings.state);
                    }
                }

                Event::MouseMotion {x, y, ..} => {
                    game.mouse_state.x = x;
                    game.mouse_state.y = y;
                }

                Event::MouseButtonDown {mouse_btn, x, y, ..} => {
                    match mouse_btn {
                        MouseButton::Left => {
                            game.mouse_state.left_pressed = true;

                            // Find the region where the mouse click occurred.
                            // If the click is within the map, generate a map click event.
                            let in_map =
                                game.display_state.zones.iter()
                                                        .filter(|zone| zone.contains(x as usize, y as usize) &&
                                                                       zone.name == "map")
                                                        .next();

                            if let Some(map_zone) = in_map {
                                let map_loc = map_zone.within(x as usize, y as usize);
                                let map_cell = (((map_loc.0 as f32 / map_zone.width as f32) * (game.data.map.width() as f32)) as i32,
                                                ((map_loc.1 as f32 / map_zone.height as f32) * (game.data.map.height() as f32)) as i32);
                                game.input_action =
                                  InputAction::MapClick(Pos::new(map_loc.0 as i32, map_loc.1 as i32),
                                                        Pos::new(map_cell.0 as i32, map_cell.1 as i32));
                            }
                        }

                        MouseButton::Middle => {
                            game.mouse_state.middle_pressed = true;
                        }

                        MouseButton::Right => {
                            game.mouse_state.right_pressed = true;
                        }

                        _ => {
                        },
                    }
                }

                Event::MouseButtonUp {mouse_btn, ..} => {
                    match mouse_btn {
                        MouseButton::Left => {
                            game.mouse_state.left_pressed = false;
                        }

                        MouseButton::Middle => {
                            game.mouse_state.middle_pressed = false;
                        }

                        MouseButton::Right => {
                            game.mouse_state.right_pressed = false;
                        }

                        _ => {},
                    }
                }

                _ => {}
            }
        }

        /* Step the Game Forward */
        let game_result = game.step_game();

        if game_result == GameResult::Stop {
            running = false;
        }

        /* Handle Message Log */
        for msg in game.msg_log.messages.iter() {
            println!("msg: {}", msg.msg_line(&game.data));

            match msg {
                Msg::StoneThrow(_thrower, stone_id, start, end) => {
                    // lay down sound objects on all tiles which can hear the sound.
                    // these dissapate with a count_down
                    let sound_aoe =
                        game.data.map.aoe_fill(map::AoeEffect::Sound, *end, SOUND_RADIUS_STONE);

                    for pos in sound_aoe.positions() {
                        let sound = generation::make_sound(&game.config, pos, *end);
                        game.data.objects.insert(sound);
                    }
                    let stone_sprite =
                        game.display_state.font_sprite(ENTITY_STONE as char)
                            .expect("Could not find stone sprite!");

                    let stone_anim = Animation::Between(stone_sprite, *start, *end, 0.0, config.stone_throw_speed);
                    let sound_anim = Animation::PlayEffect(Effect::Sound(sound_aoe, 0.0));
                    let loop_anim = Animation::Loop(stone_sprite);

                    let stone_key = game.display_state.play_animation(stone_anim);
                    let sound_key = game.display_state.play_animation(sound_anim);
                    let loop_key = game.display_state.play_animation(loop_anim);

                    game.data.objects[*stone_id].animation.clear();
                    game.data.objects[*stone_id].animation.push_back(stone_key);
                    game.data.objects[*stone_id].animation.push_back(sound_key);
                    game.data.objects[*stone_id].animation.push_back(loop_key);
                }

                Msg::Moved(object_id, movement, pos) => {
                    let player_handle = game.data.find_player().unwrap();
                    if *object_id == player_handle {

                        if let Movement::Pass(_) = movement {
                            if game.data.objects[player_handle].move_mode.unwrap() ==
                               MoveMode::Run {
                                let player = &mut game.data.objects[player_handle];
                                player.move_mode = player.move_mode.map(|mode| mode.decrease());
                            }
                            // this is just to pattern match on movement
                        } else {
                            let sound_radius;
                            match game.data.objects[player_handle].move_mode.unwrap() {
                                MoveMode::Sneak => sound_radius = SOUND_RADIUS_SNEAK,
                                MoveMode::Walk => sound_radius = SOUND_RADIUS_WALK,
                                MoveMode::Run => sound_radius = SOUND_RADIUS_RUN,
                            }

                            let idle_sprite =
                                game.display_state.new_sprite("player_idle".to_string(), config.idle_speed)
                                                  .unwrap();
                            let idle_anim = Animation::Loop(idle_sprite);
                            let idle_key = game.display_state.play_animation(idle_anim);

                            game.data.objects[player_handle].animation.clear();
                            game.data.objects[player_handle].animation.push_back(idle_key);

                            // add sound for movement
                            let sound_aoe =
                                game.data.map.aoe_fill(map::AoeEffect::Sound, *pos, sound_radius);
                            for sound_pos in sound_aoe.positions() {
                                let sound = generation::make_sound(&game.config, sound_pos, *pos);
                                game.data.objects.insert(sound);
                            }

                            let sound_effect = Effect::Sound(sound_aoe, 0.0);
                            game.display_state.play_effect(sound_effect);
                        }
                    }
                }

                Msg::Yell(_pos) => {
                    let player_handle = game.data.find_player().unwrap();
                    let player_pos = game.data.objects[player_handle].pos();

                    let sound_aoe =
                        game.data.map.aoe_fill(map::AoeEffect::Sound, player_pos, config.player_yell_radius);
                    for pos in sound_aoe.positions() {
                        let sound = generation::make_sound(&game.config, pos, player_pos);
                        game.data.objects.insert(sound);
                    }

                    let sound_effect = Effect::Sound(sound_aoe, 0.0);
                    game.display_state.play_effect(sound_effect);
                }

                Msg::Killed(_attacker, attacked, _damage) => {
                    if game.data.objects[*attacked].name != "player".to_string() {
                        game.data.objects[*attacked].animation.clear();

                        if game.data.objects[*attacked].name == "gol".to_string() {
                            let gol_sprite = game.display_state.new_sprite("gol_die".to_string(), 1.0)
                                                               .unwrap();
                            let gol_anim = game.display_state.play_animation(Animation::Once(gol_sprite));
                            game.data.objects[*attacked].animation.clear();
                            game.data.objects[*attacked].animation.push_front(gol_anim);
                        }
                    }
                }

                Msg::Attack(attacker, _attacked, _damage) => {
                    if game.data.objects[*attacker].name == "player" {
                        let attack_sprite =
                            game.display_state.new_sprite("player_attack".to_string(), config.player_attack_speed)
                                              .unwrap();
                        let attack_anim = Animation::Once(attack_sprite);
                        let attack_key = game.display_state.play_animation(attack_anim);

                        let idle_sprite =
                            game.display_state.new_sprite("player_idle".to_string(), config.idle_speed)
                                              .unwrap();
                        let idle_anim = Animation::Loop(idle_sprite);
                        let idle_key = game.display_state.play_animation(idle_anim);

                        game.data.objects[*attacker].animation.clear();
                        game.data.objects[*attacker].animation.push_back(attack_key);
                        game.data.objects[*attacker].animation.push_back(idle_key);
                    }
                }

                _ => {
                }
            }
        }
        game.msg_log.clear();

        /* Draw the Game to the Screen */
        render_all(&mut game.display_state,
                   &mut game.mouse_state,
                   &mut game.data,
                   &game.settings,
                   &game.config)?;

        /* Reload map if configured to do so */
        if game.config.load_map_file_every_frame && Path::new("resources/map.xp").exists() {
            let player_handle = game.data.find_player().unwrap();

            let map_file = format!("resources/{}", game.config.map_file);
            let (new_objects, new_map, _) = read_map_xp(&game.config, &mut game.display_state, &map_file);
            game.data.map = new_map;
            game.data.objects[player_handle].inventory.clear();
            let player = game.data.objects[player_handle].clone();
            game.data.objects.clear();
            for key in new_objects.keys() {
                game.data.objects.insert(new_objects[key].clone());
            }
            game.data.objects.insert(player);
        }

        /* Reload Configuration */
        match File::open("config.json") {
            Ok(mut file) => {
                let mut config_string = String::new();
                file.read_to_string(&mut config_string).expect("Could not read config file!");
                game.config = serde_json::from_str(&config_string).expect("Could not read JSON- config.json has a parsing error!");
            }
            _ => (),
        }

        /* Wait until the next tick to loop */
        fps_throttler.wait();
    }

    return Ok(());
}

