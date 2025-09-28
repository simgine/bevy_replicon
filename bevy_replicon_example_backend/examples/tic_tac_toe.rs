//! Tic-tac-toe game with optional multiplayer.
//! Client sends commands and server replicates the state back.

use std::{
    fmt::{self, Formatter},
    net::{IpAddr, Ipv4Addr},
};

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
                    resolution: (800, 600).into(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            RepliconPlugins,
            RepliconExampleBackendPlugins,
        ))
        .init_state::<GameState>()
        .init_resource::<SymbolFont>()
        .init_resource::<TurnSymbol>()
        .replicate::<Symbol>()
        .add_client_trigger::<CellPick>(Channel::Ordered)
        .insert_resource(ClearColor(BACKGROUND_COLOR))
        .add_observer(disconnect_by_client)
        .add_observer(init_client)
        .add_observer(apply_pick)
        .add_observer(init_symbols)
        .add_observer(advance_turn)
        .add_systems(Startup, (read_cli, setup_ui))
        .add_systems(
            OnEnter(GameState::InGame),
            (show_turn_text, show_turn_symbol),
        )
        .add_systems(OnEnter(GameState::Disconnected), show_disconnected_text)
        .add_systems(OnEnter(GameState::Winner), show_winner_text)
        .add_systems(OnEnter(GameState::Tie), show_tie_text)
        .add_systems(OnEnter(GameState::Disconnected), stop_networking)
        .add_systems(OnEnter(ClientState::Connected), client_start)
        .add_systems(OnEnter(ClientState::Connecting), show_connecting_text)
        .add_systems(OnExit(ClientState::Connected), disconnect_by_server)
        .add_systems(OnEnter(ServerState::Running), show_waiting_client_text)
        .add_systems(
            Update,
            (
                update_buttons_background.run_if(local_player_turn),
                show_turn_symbol.run_if(resource_changed::<TurnSymbol>),
            )
                .run_if(in_state(GameState::InGame)),
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

            // Backend initialization
            let server = ExampleServer::new(port)?;
            commands.insert_resource(server);

            commands.spawn((LocalPlayer, symbol));
        }
        Cli::Client { ip, port } => {
            info!("connecting to {ip}:{port}");

            // Backend initialization
            let client = ExampleClient::new((ip, port))?;
            commands.insert_resource(client);

            commands.spawn((LocalPlayer, ClientPlayer));
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
    click: On<Pointer<Click>>,
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
        .get(click.entity)
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
    pick: On<FromClient<CellPick>>,
    mut commands: Commands,
    cells: Query<(Entity, &Cell), Without<Symbol>>,
    turn_symbol: Res<TurnSymbol>,
    players: Query<&Symbol>,
) {
    // It's good to check the received data because client could be cheating.
    if let ClientId::Client(client) = pick.client_id {
        let symbol = *players
            .get(client)
            .expect("all clients should have assigned symbols");
        if symbol != **turn_symbol {
            error!("`{client}` chose cell {} at wrong turn", pick.index);
            return;
        }
    }

    let Some((entity, _)) = cells.iter().find(|(_, cell)| cell.index == pick.index) else {
        error!(
            "`{}` has chosen occupied or invalid cell {}",
            pick.client_id, pick.index
        );
        return;
    };

    commands.entity(entity).insert(**turn_symbol);
}

/// Initializes spawned symbol on client after replication and on server / single-player right after the spawn.
fn init_symbols(
    add: On<Add, Symbol>,
    mut commands: Commands,
    symbol_font: Res<SymbolFont>,
    mut cells: Query<(&mut BackgroundColor, &Symbol), With<Button>>,
) {
    let Ok((mut background, symbol)) = cells.get_mut(add.entity) else {
        return;
    };
    *background = BACKGROUND_COLOR.into();

    commands
        .entity(add.entity)
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

/// Starts the game after connection.
///
/// Used only for a client.
fn client_start(mut commands: Commands) {
    commands.set_state(GameState::InGame);
}

/// Associates client with a symbol and starts the game.
///
/// Used only for server.
fn init_client(
    add: On<Add, AuthorizedClient>,
    mut commands: Commands,
    server_symbol: Single<&Symbol, With<LocalPlayer>>,
) {
    // Utilize client entity as a player for convenient lookups by `client`.
    commands.entity(add.entity).insert((
        ClientPlayer,
        Signature::of::<ClientPlayer>(),
        server_symbol.next(),
    ));

    commands.set_state(GameState::InGame);
}

/// Sets the game in disconnected state if client closes the connection.
///
/// Used only for server.
fn disconnect_by_client(
    _on: On<Remove, ConnectedClient>,
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
    _on: On<Add, Symbol>,
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

const DEFAULT_PORT: u16 = 5000;

/// A Tic-tac-toe game.
#[derive(Parser, PartialEq, Resource)]
enum Cli {
    /// Play locally.
    Hotseat,
    /// Create a server that acts as both player and host.
    Server {
        #[arg(short, long, default_value_t = DEFAULT_PORT)]
        port: u16,

        #[arg(short, long, default_value_t = Symbol::Cross)]
        symbol: Symbol,
    },
    /// Connect to a host.
    Client {
        #[arg(short, long, default_value_t = Ipv4Addr::LOCALHOST.into())]
        ip: IpAddr,

        #[arg(short, long, default_value_t = DEFAULT_PORT)]
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

/// The player's symobol, current [`TurnSymbol`] or a symbol of a filled cell (see [`CellPick`]).
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
/// We want to replicate all cells, so we set [`Replicated`] as a required component.
/// We also want entities with this component to be automatically mapped between
/// client and server, so we also require the [`Signature`] component.
#[derive(Component, Hash)]
#[require(
    Button,
    Replicated,
    BackgroundColor(BACKGROUND_COLOR),
    Signature::of::<Cell>(),
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

/// Player that can be controlled from the current machine.
///
/// Used to determine if player can place a symbol.
/// See also [`local_player_turn`].
#[derive(Component)]
#[require(Replicated)]
struct LocalPlayer;

/// Player that is also a client.
///
/// Used to spawn an entity with [`LocalPlayer`] on the client
/// and automatically map it to the player entity on the server
/// with the [`Signature`] component.
#[derive(Component, Hash)]
#[require(Replicated, Signature::of::<ClientPlayer>())]
struct ClientPlayer;

/// A trigger that indicates a symbol pick.
///
/// We don't replicate the whole UI, so we can't just send the picked entity because on server it may be different.
/// So we send the cell location in grid and calculate the entity on server based on this.
#[derive(Clone, Copy, Deserialize, Event, Serialize)]
struct CellPick {
    index: usize,
}
