//! A game to showcase single-player and multiplier game.
//! Run it with `cargo run --example tic_tac_toe -- hotseat` to play locally or with `-- client` / `-- server`

use std::fmt::{self, Formatter};

use bevy::{
    ecs::{relationship::RelatedSpawner, spawn::SpawnWith},
    prelude::*,
};
use bevy_replicon::prelude::*;
use bevy_replicon_example_backend::{ExampleClient, ExampleServer, RepliconExampleBackendPlugins};
use clap::{Parser, ValueEnum};
use serde::{Deserialize, Serialize};

fn main() {
    App::new()
        .init_resource::<Cli>() // Parse CLI before creating window.
        .add_plugins((
            DefaultPlugins.build().set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Tic-Tac-Toe".into(),
                    resolution: (800.0, 600.0).into(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            RepliconPlugins.set(RepliconSharedPlugin {
                // Customize authorization because we want to exchange cell mappings first.
                auth_method: AuthMethod::Custom,
            }),
            RepliconExampleBackendPlugins,
        ))
        .init_state::<GameState>()
        .init_resource::<SymbolFont>()
        .init_resource::<TurnSymbol>()
        .replicate::<Symbol>()
        .add_client_trigger::<ClientInfo>(Channel::Ordered)
        .add_client_trigger::<CellPick>(Channel::Ordered)
        .add_server_trigger::<MakeLocal>(Channel::Ordered)
        .insert_resource(ClearColor(BACKGROUND_COLOR))
        .add_observer(disconnect_by_client)
        .add_observer(init_client)
        .add_observer(make_local)
        .add_observer(apply_pick)
        .add_observer(init_symbols)
        .add_observer(advance_turn)
        .add_systems(Startup, (setup_ui, read_cli))
        .add_systems(
            OnEnter(GameState::InGame),
            (show_turn_text, show_turn_symbol),
        )
        .add_systems(OnEnter(GameState::Disconnected), show_disconnected_text)
        .add_systems(OnEnter(GameState::Winner), show_winner_text)
        .add_systems(OnEnter(GameState::Tie), show_tie_text)
        .add_systems(OnEnter(GameState::Disconnected), stop_networking)
        .add_systems(
            Update,
            (
                show_connecting_text.run_if(resource_added::<ExampleClient>),
                show_waiting_client_text.run_if(resource_added::<ExampleServer>),
                client_start.run_if(client_just_connected),
                (
                    disconnect_by_server.run_if(client_just_disconnected),
                    update_buttons_background.run_if(local_player_turn),
                    show_turn_symbol.run_if(resource_changed::<TurnSymbol>),
                )
                    .run_if(in_state(GameState::InGame)),
            ),
        )
        .run();
}

const GRID_SIZE: usize = 3;

const BACKGROUND_COLOR: Color = Color::srgb(0.9, 0.9, 0.9);

// Bottom text defined in two sections, first for text and second for symbols with different font.
const TEXT_SECTION: usize = 0;
const SYMBOL_SECTION: usize = 1;

const CELL_SIZE: f32 = 100.0;
const LINE_THICKNESS: f32 = 10.0;

const BUTTON_SIZE: f32 = CELL_SIZE / 1.2;
const BUTTON_MARGIN: f32 = (CELL_SIZE + LINE_THICKNESS - BUTTON_SIZE) / 2.0;

fn read_cli(mut commands: Commands, cli: Res<Cli>) -> Result<()> {
    match *cli {
        Cli::Hotseat => {
            info!("starting hotseat");
            // Set all players to server to play from a single machine and start the game right away.
            commands.spawn((LocalPlayer, Symbol::Cross));
            commands.spawn((LocalPlayer, Symbol::Nought));
            commands.set_state(GameState::InGame);
        }
        Cli::Server { port, symbol } => {
            info!("starting server as {symbol} at port {port}");
            let server = ExampleServer::new(port)?;
            commands.insert_resource(server);
            commands.spawn((LocalPlayer, symbol));
        }
        Cli::Client { port } => {
            info!("connecting to port {port}");
            let client = ExampleClient::new(port)?;
            commands.insert_resource(client);
        }
    }

    Ok(())
}

fn setup_ui(mut commands: Commands, symbol_font: Res<SymbolFont>) {
    commands.spawn(Camera2d);

    const LINES_COUNT: usize = GRID_SIZE + 1;
    const BOARD_SIZE: f32 = CELL_SIZE * GRID_SIZE as f32 + LINES_COUNT as f32 * LINE_THICKNESS;
    const BOARD_COLOR: Color = Color::srgb(0.8, 0.8, 0.8);

    for line in 0..LINES_COUNT {
        let position =
            -BOARD_SIZE / 2.0 + line as f32 * (CELL_SIZE + LINE_THICKNESS) + LINE_THICKNESS / 2.0;

        // Horizontal
        commands.spawn((
            Sprite {
                color: BOARD_COLOR,
                ..Default::default()
            },
            Transform {
                translation: Vec3::Y * position,
                scale: Vec3::new(BOARD_SIZE, LINE_THICKNESS, 1.0),
                ..Default::default()
            },
        ));

        // Vertical
        commands.spawn((
            Sprite {
                color: BOARD_COLOR,
                ..Default::default()
            },
            Transform {
                translation: Vec3::X * position,
                scale: Vec3::new(LINE_THICKNESS, BOARD_SIZE, 1.0),
                ..Default::default()
            },
        ));
    }

    const TEXT_COLOR: Color = Color::srgb(0.5, 0.5, 1.0);
    const FONT_SIZE: f32 = 32.0;

    commands.spawn((
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..Default::default()
        },
        children![(
            Node {
                flex_direction: FlexDirection::Column,
                width: Val::Px(BOARD_SIZE - LINE_THICKNESS),
                height: Val::Px(BOARD_SIZE - LINE_THICKNESS),
                ..Default::default()
            },
            children![
                (
                    Node {
                        display: Display::Grid,
                        grid_template_columns: vec![GridTrack::auto(); GRID_SIZE],
                        ..Default::default()
                    },
                    Children::spawn(SpawnWith(|parent: &mut RelatedSpawner<_>| {
                        for index in 0..GRID_SIZE * GRID_SIZE {
                            parent.spawn(Cell { index }).observe(pick_cell);
                        }
                    }))
                ),
                (
                    Node {
                        margin: UiRect::top(Val::Px(20.0)),
                        justify_content: JustifyContent::Center,
                        ..Default::default()
                    },
                    children![(
                        Text::default(),
                        TextFont {
                            font_size: FONT_SIZE,
                            ..Default::default()
                        },
                        TextColor(TEXT_COLOR),
                        BottomText,
                        children![(
                            TextSpan::default(),
                            TextFont {
                                font: symbol_font.0.clone(),
                                font_size: FONT_SIZE,
                                ..Default::default()
                            },
                            TextColor(TEXT_COLOR),
                        )]
                    )]
                )
            ]
        )],
    ));
}

/// Converts point clicks into cell picking events.
///
/// We don't just send mouse clicks to save traffic, they contain a lot of extra information.
fn pick_cell(
    trigger: Trigger<Pointer<Click>>,
    mut commands: Commands,
    turn_symbol: Res<TurnSymbol>,
    game_state: Res<State<GameState>>,
    cells: Query<&Cell>,
    players: Query<&Symbol, With<LocalPlayer>>,
) {
    if *game_state != GameState::InGame {
        return;
    }

    if !local_player_turn(turn_symbol, players) {
        return;
    }

    let cell = cells
        .get(trigger.target())
        .expect("cells should have assigned indices");
    // We don't check if a cell can't be picked on client on purpose
    // just to demonstrate how server can receive invalid requests from a client.
    info!("picking cell {}", cell.index);
    commands.client_trigger(CellPick { index: cell.index });
}

/// Handles cell pick events.
///
/// Used only for single-player and server.
fn apply_pick(
    trigger: Trigger<FromClient<CellPick>>,
    mut commands: Commands,
    cells: Query<(Entity, &Cell), Without<Symbol>>,
    turn_symbol: Res<TurnSymbol>,
    players: Query<&Symbol>,
) {
    // It's good to check the received data because client could be cheating.
    if let ClientId::Client(client) = trigger.client_id {
        let symbol = *players
            .get(client)
            .expect("all clients should have assigned symbols");
        if symbol != **turn_symbol {
            error!("`{client}` chose cell {} at wrong turn", trigger.index);
            return;
        }
    }

    let Some((entity, _)) = cells.iter().find(|(_, cell)| cell.index == trigger.index) else {
        error!(
            "`{}` has chosen occupied or invalid cell {}",
            trigger.client_id, trigger.index
        );
        return;
    };

    commands.entity(entity).insert(**turn_symbol);
}

/// Initializes spawned symbol on client after replication and on server / single-player right after the spawn.
fn init_symbols(
    trigger: Trigger<OnAdd, Symbol>,
    mut commands: Commands,
    symbol_font: Res<SymbolFont>,
    mut cells: Query<(&mut BackgroundColor, &Symbol), With<Button>>,
) {
    let Ok((mut background, symbol)) = cells.get_mut(trigger.target()) else {
        return;
    };
    *background = BACKGROUND_COLOR.into();

    commands
        .entity(trigger.target())
        .remove::<Interaction>()
        .with_child((
            Text::new(symbol.glyph()),
            TextFont {
                font: symbol_font.0.clone(),
                font_size: 65.0,
                ..Default::default()
            },
            TextColor(symbol.color()),
        ));
}

/// Sends cell and local player entities and starts the game.
///
/// Replicon maps entities when you replicate them from server automatically.
/// But in this game we spawn cells beforehand. So we send a special event to
/// server to receive replication to already existing entities.
///
/// Used only for client.
fn client_start(
    mut commands: Commands,
    protocol: Res<ProtocolHash>,
    cells: Query<(Entity, &Cell)>,
) {
    let mut cells: Vec<_> = cells.iter().collect();
    cells.sort_by_key(|(_, cell)| cell.index);

    commands.client_trigger(ClientInfo {
        protocol: *protocol,
        cells: cells.into_iter().map(|(entity, _)| entity).collect(),
    });
    commands.set_state(GameState::InGame);
}

/// Establishes mappings between spawned client and server entities and starts the game.
///
/// Used only for server.
fn init_client(
    trigger: Trigger<FromClient<ClientInfo>>,
    mut commands: Commands,
    mut events: EventWriter<DisconnectRequest>,
    protocol: Res<ProtocolHash>,
    cells: Query<(Entity, &Cell)>,
    server_symbol: Single<&Symbol, With<LocalPlayer>>,
) {
    let client = trigger
        .client_id
        .entity()
        .expect("protocol hash sent only from clients");

    // Since we using custom authorization,
    // we need to verify the protocol manually.
    if trigger.protocol != *protocol {
        // Notify client about the problem. No delivery
        // guarantee since we disconnect after sending.
        commands.server_trigger(ToClients {
            mode: SendMode::Direct(trigger.client_id),
            event: ProtocolMismatch,
        });
        events.write(DisconnectRequest { client });
    }

    // Sort local square entities to match them with the received.
    let mut cells: Vec<_> = cells.iter().collect();
    cells.sort_by_key(|(_, cell)| cell.index);

    // This map is a required component for `AuthorizedClient`.
    // By default it's empty, but we can initialize it with the
    // received entities.
    let mut entity_map = ClientEntityMap::default();
    for (&server, &client) in cells.iter().map(|(entity, _)| entity).zip(&trigger.cells) {
        entity_map.insert(server, client);
    }

    // Utilize client entity as a player for convenient lookups by `client`.
    commands
        .entity(client)
        .insert((Player, server_symbol.next(), AuthorizedClient, entity_map));

    commands.server_trigger_targets(
        ToClients {
            mode: SendMode::Direct(trigger.client_id),
            event: MakeLocal,
        },
        client,
    );

    commands.set_state(GameState::InGame);
}

fn make_local(trigger: Trigger<MakeLocal>, mut commands: Commands) {
    commands.entity(trigger.target()).insert(LocalPlayer);
}

/// Sets the game in disconnected state if client closes the connection.
///
/// Used only for server.
fn disconnect_by_client(
    _trigger: Trigger<OnRemove, ConnectedClient>,
    game_state: Res<State<GameState>>,
    mut commands: Commands,
) {
    info!("client closed the connection");
    if *game_state == GameState::InGame {
        commands.set_state(GameState::Disconnected);
    }
}

/// Sets the game in disconnected state if server closes the connection.
///
/// Used only for client.
fn disconnect_by_server(mut commands: Commands) {
    info!("server closed the connection");
    commands.set_state(GameState::Disconnected);
}

/// Closes all sockets.
fn stop_networking(mut commands: Commands) {
    commands.remove_resource::<ExampleServer>();
    commands.remove_resource::<ExampleClient>();
}

/// Checks the winner and advances the turn.
fn advance_turn(
    _trigger: Trigger<OnAdd, Symbol>,
    mut commands: Commands,
    mut turn_symbol: ResMut<TurnSymbol>,
    symbols: Query<(&Cell, &Symbol)>,
) {
    let mut board = [None; GRID_SIZE * GRID_SIZE];
    for (cell, &symbol) in &symbols {
        board[cell.index] = Some(symbol);
    }

    const WIN_CONDITIONS: [[usize; GRID_SIZE]; 8] = [
        [0, 1, 2],
        [3, 4, 5],
        [6, 7, 8],
        [0, 3, 6],
        [1, 4, 7],
        [2, 5, 8],
        [0, 4, 8],
        [2, 4, 6],
    ];

    for indices in WIN_CONDITIONS {
        let symbols = indices.map(|index| board[index]);
        if symbols[0].is_some() && symbols.windows(2).all(|symbols| symbols[0] == symbols[1]) {
            commands.set_state(GameState::Winner);
            info!("{} wins the game", **turn_symbol);
            return;
        }
    }

    if board.iter().all(Option::is_some) {
        info!("game ended in a tie");
        commands.set_state(GameState::Tie);
    } else {
        **turn_symbol = turn_symbol.next();
    }
}

fn update_buttons_background(
    mut buttons: Query<(&Interaction, &mut BackgroundColor), Changed<Interaction>>,
) {
    const HOVER_COLOR: Color = Color::srgb(0.85, 0.85, 0.85);
    const PRESS_COLOR: Color = Color::srgb(0.95, 0.95, 0.95);

    for (interaction, mut background) in &mut buttons {
        match interaction {
            Interaction::Pressed => *background = PRESS_COLOR.into(),
            Interaction::Hovered => *background = HOVER_COLOR.into(),
            Interaction::None => *background = BACKGROUND_COLOR.into(),
        };
    }
}

fn show_turn_text(mut writer: TextUiWriter, text: Single<Entity, With<BottomText>>) {
    *writer.text(*text, TEXT_SECTION) = "Current turn: ".into();
}

fn show_turn_symbol(
    mut writer: TextUiWriter,
    turn_symbol: Res<TurnSymbol>,
    text: Single<Entity, With<BottomText>>,
) {
    *writer.text(*text, SYMBOL_SECTION) = turn_symbol.glyph().into();
    *writer.color(*text, SYMBOL_SECTION) = turn_symbol.color().into();
}

fn show_disconnected_text(mut writer: TextUiWriter, text: Single<Entity, With<BottomText>>) {
    *writer.text(*text, TEXT_SECTION) = "Disconnected".into();
    writer.text(*text, SYMBOL_SECTION).clear();
}

fn show_winner_text(mut writer: TextUiWriter, text: Single<Entity, With<BottomText>>) {
    *writer.text(*text, TEXT_SECTION) = "Winner: ".into();
}

fn show_tie_text(mut writer: TextUiWriter, text: Single<Entity, With<BottomText>>) {
    *writer.text(*text, TEXT_SECTION) = "Tie".into();
    writer.text(*text, SYMBOL_SECTION).clear();
}

fn show_connecting_text(mut writer: TextUiWriter, text: Single<Entity, With<BottomText>>) {
    *writer.text(*text, TEXT_SECTION) = "Connecting".into();
}

fn show_waiting_client_text(mut writer: TextUiWriter, text: Single<Entity, With<BottomText>>) {
    *writer.text(*text, TEXT_SECTION) = "Waiting client".into();
}

/// Returns `true` if the local player can select cells.
fn local_player_turn(
    turn_symbol: Res<TurnSymbol>,
    players: Query<&Symbol, With<LocalPlayer>>,
) -> bool {
    players.iter().any(|&symbol| symbol == **turn_symbol)
}

const PORT: u16 = 5000;

#[derive(Parser, PartialEq, Resource)]
enum Cli {
    Hotseat,
    Server {
        #[arg(short, long, default_value_t = PORT)]
        port: u16,

        #[arg(short, long, default_value_t = Symbol::Cross)]
        symbol: Symbol,
    },
    Client {
        #[arg(short, long, default_value_t = PORT)]
        port: u16,
    },
}

impl Default for Cli {
    fn default() -> Self {
        Self::parse()
    }
}

/// Font to display unicode characters for [`Symbol`].
#[derive(Resource)]
struct SymbolFont(Handle<Font>);

impl FromWorld for SymbolFont {
    fn from_world(world: &mut World) -> Self {
        let asset_server = world.resource::<AssetServer>();
        Self(asset_server.load("NotoEmoji-Regular.ttf"))
    }
}

#[derive(States, Clone, Copy, Debug, Eq, Hash, PartialEq, Default)]
enum GameState {
    #[default]
    WaitingPlayer,
    InGame,
    Winner,
    Tie,
    Disconnected,
}

/// Contains symbol to be used this turn.
#[derive(Resource, Default, Deref, DerefMut)]
struct TurnSymbol(Symbol);

/// A component that defines the symbol of a [`Player`], current [`TurnSymbol`] or a filled cell (see [`CellPick`]).
#[derive(Clone, Component, Copy, Default, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
enum Symbol {
    #[default]
    Cross,
    Nought,
}

impl Symbol {
    fn glyph(self) -> &'static str {
        match self {
            Symbol::Cross => "❌",
            Symbol::Nought => "⭕",
        }
    }

    fn color(self) -> Color {
        match self {
            Symbol::Cross => Color::srgb(1.0, 0.5, 0.5),
            Symbol::Nought => Color::srgb(0.5, 0.5, 1.0),
        }
    }

    fn next(self) -> Self {
        match self {
            Symbol::Cross => Symbol::Nought,
            Symbol::Nought => Symbol::Cross,
        }
    }
}

impl fmt::Display for Symbol {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Symbol::Cross => f.write_str("cross"),
            Symbol::Nought => f.write_str("nought"),
        }
    }
}

/// Marker for UI node with bottom text.
#[derive(Component)]
struct BottomText;

/// Cell location on the grid.
///
/// We want to replicate all cells, so we just set [`Replicated`] as a required component.
#[derive(Component)]
#[require(
    Button,
    Replicated,
    BackgroundColor(BACKGROUND_COLOR),
    Node {
        width: Val::Px(BUTTON_SIZE),
        height: Val::Px(BUTTON_SIZE),
        margin: UiRect::all(Val::Px(BUTTON_MARGIN)),
        ..Default::default()
    }
)]
struct Cell {
    index: usize,
}

/// Marker for a player entity.
#[derive(Component, Default)]
#[require(Replicated)]
struct Player;

/// Marks [`Player`] as locally controlled.
///
/// Used to determine if player can place a symbol.
///
/// See also [`local_player_turn`].
#[derive(Component)]
#[require(Player)]
struct LocalPlayer;

/// A trigger that indicates a symbol pick.
///
/// We don't replicate the whole UI, so we can't just send the picked entity because on server it may be different.
/// So we send the cell location in grid and calculate the entity on server based on this.
#[derive(Clone, Copy, Deserialize, Event, Serialize)]
struct CellPick {
    index: usize,
}

/// A client trigger with protocol information and client's chess board entities.
///
/// See [`client_start`] for details.
#[derive(Event, Serialize, Deserialize)]
struct ClientInfo {
    protocol: ProtocolHash,
    cells: Vec<Entity>,
}

/// A trigger that instructs the client to mark a specific entity as [`LocalPlayer`].
#[derive(Event, Serialize, Deserialize)]
struct MakeLocal;
