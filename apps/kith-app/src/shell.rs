//! The shell: the native tray menu and icon, the window and its page server, and the
//! notifications, all kept from one model of what the page server last said.
//!
//! Tauri calls are never made while the state lock is held: menu and window calls hop to
//! the main thread, and a callback there may need the lock.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use kith_ui_core::Source;
use kith_ui_core::wire::Fixture;
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu as NativeMenu, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::webview::PageLoadEvent;
use tauri::window::Color;
use tauri::{
    AppHandle, Manager as _, RunEvent, Theme, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
    WindowEvent,
};

use crate::client::Client;
use crate::copy;
use crate::deeplink::join_code;
use crate::icon::{self, IconState};
use crate::model::{self, Menu, Reach};
use crate::notify::{self, Gate, Notice, RecentUse};
use crate::page::{self, Bound, Page};
use crate::timing::{DAEMON_STOPPED_NOTIFY_MS, WINDOW_SHOW_FALLBACK};
use crate::tokens::{BG_DARK, BG_LIGHT, Rgb};

/// The tray icon's id.
const TRAY: &str = "kith";
/// The tray icon's side in pixels: the menu bar's 18 pt at 2×, which other panels scale.
const ICON_SIZE: u32 = 36;
/// The window's first size, in logical pixels: the spec's desktop frame.
const WINDOW_SIZE: (f64, f64) = (1280.0, 800.0);
/// The smallest window: the spec's narrowest layout.
const WINDOW_MIN: (f64, f64) = (360.0, 480.0);

/// Where the shell's data comes from.
#[derive(Debug, Clone)]
pub(crate) enum Origin {
    /// The daemon on this socket.
    Daemon(PathBuf),
    /// A recorded network, for building and testing without a daemon.
    Fixture(Arc<Fixture>),
}

impl Origin {
    /// A fresh source for one router.
    pub(crate) fn source(&self) -> Source {
        match self {
            Self::Daemon(socket) => Source::Daemon(socket.clone()),
            Self::Fixture(fixture) => Source::Fixture(Box::new(Fixture::clone(fixture))),
        }
    }
}

/// The open window and the page server behind it.
#[derive(Debug)]
struct Open {
    /// The window's label, unique per opening.
    label: String,
    /// Its page server; dropping it stops the server.
    page: Page,
}

/// What the shell knows, behind one lock.
#[derive(Debug, Default)]
struct State {
    reach: Option<Reach>,
    menu: Option<Menu>,
    used: RecentUse,
    gate: Gate,
    /// When the daemon was first seen down, by this machine's clock.
    down_since: Option<u64>,
    /// Whether that stop has notified.
    down_told: bool,
    client: Option<Client>,
    window: Option<Open>,
    /// Counts windows opened, for unique labels.
    opened: u64,
    /// A window is being opened; another request only waits for it.
    opening: bool,
}

/// The shell, shared by the tray, the window, the watcher and the notifications.
#[derive(Debug)]
pub(crate) struct Shell {
    app: AppHandle,
    origin: Origin,
    state: Mutex<State>,
}

impl Shell {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The data source for a new router.
    pub(crate) fn source(&self) -> Source {
        self.origin.source()
    }

    /// Keeps the tray's client, for the exit choices.
    pub(crate) fn set_client(&self, client: Client) {
        self.state().client = Some(client);
    }

    fn window(&self) -> Option<WebviewWindow> {
        let label = self
            .state()
            .window
            .as_ref()
            .map(|open| open.label.clone())?;
        self.app.get_webview_window(&label)
    }

    fn focused(&self) -> bool {
        self.window()
            .is_some_and(|window| window.is_focused().unwrap_or(false))
    }

    /// Takes what the page server said: redraws the tray when it changed and notifies
    /// what the rules let through.
    pub(crate) fn apply(self: &Arc<Self>, reach: Reach) {
        let now = now_ms();
        let focused = self.focused();
        let (redraw, shown) = {
            let mut state = self.state();
            let notices = match (&state.reach, &reach) {
                (before, Reach::Up(after)) => {
                    let before = match before {
                        Some(Reach::Up(before)) => Some(before.as_ref()),
                        Some(Reach::Down(_)) | None => None,
                    };
                    notify::notices(before, after, &state.used)
                }
                (_, Reach::Down(_)) => Vec::new(),
            };
            if matches!(reach, Reach::Down(_)) {
                state.down_since.get_or_insert(now);
            } else {
                state.down_since = None;
                state.down_told = false;
            }
            let menu = model::menu(&reach);
            state.reach = Some(reach);
            let redraw = (state.menu.as_ref() != Some(&menu)).then(|| menu.clone());
            state.menu = Some(menu);
            let shown: Vec<Notice> = notices
                .into_iter()
                .filter_map(|notice| state.gate.admit(notice, now, focused))
                .collect();
            (redraw, shown)
        };
        if let Some(menu) = redraw {
            self.draw(menu);
        }
        for notice in shown {
            self.notify(notice);
        }
    }

    /// Takes a tick's traffic counters, then does what is due by the clock.
    pub(crate) fn tick(self: &Arc<Self>, tick: &kith_ui_core::wire::Tick) {
        self.state().used.tick(tick);
        self.housekeeping();
    }

    /// Notifies a daemon stopped for long enough, and lets out held notifications.
    pub(crate) fn housekeeping(self: &Arc<Self>) {
        let now = now_ms();
        let focused = self.focused();
        let shown: Vec<Notice> = {
            let mut state = self.state();
            let stopped = state.down_since.is_some_and(|since| {
                !state.down_told && now.saturating_sub(since) >= DAEMON_STOPPED_NOTIFY_MS
            });
            let mut shown = Vec::new();
            if stopped {
                state.down_told = true;
                shown.extend(state.gate.admit(notify::daemon_stopped(), now, focused));
            }
            shown.extend(state.gate.flush(now, focused));
            shown
        };
        for notice in shown {
            self.notify(notice);
        }
    }

    /// Shows `notice`; `Review` opens its page.
    fn notify(self: &Arc<Self>, notice: Notice) {
        let shell = Arc::clone(self);
        let identifier = self.app.config().identifier.clone();
        std::thread::spawn(move || {
            if notify::show_and_wait(&identifier, &notice) {
                shell.open(Some(notice.target), None);
            }
        });
    }

    /// Redraws the tray menu and icon.
    fn draw(&self, menu: Menu) {
        let app = self.app.clone();
        let _gone = self.app.run_on_main_thread(move || {
            if let Some(tray) = app.tray_by_id(TRAY) {
                let _not_drawn = draw_tray(&app, &tray, &menu);
            }
        });
    }

    /// Handles a tray menu item.
    pub(crate) fn on_menu(self: &Arc<Self>, id: &str) {
        let menu = self.state().menu.clone();
        let Some(menu) = menu else { return };
        match id {
            "open" => self.open(None, None),
            "status" => self.open(Some(menu.home), None),
            "needs" => self.open(menu.needs.map(|needs| needs.target), None),
            "quit" => self.app.exit(0),
            other => {
                if let Some(index) = other.strip_prefix("exit:").and_then(|i| i.parse().ok()) {
                    self.choose_exit(menu, index);
                }
            }
        }
    }

    /// Switches exit: the tray shows the choice at once and goes back if it fails.
    fn choose_exit(self: &Arc<Self>, mut menu: Menu, index: usize) {
        let Some(chosen) = menu.exits.get(index).cloned() else {
            return;
        };
        let current = menu
            .exits
            .iter()
            .find(|exit| exit.checked)
            .and_then(|exit| exit.name.clone());
        for (at, exit) in menu.exits.iter_mut().enumerate() {
            exit.checked = at == index;
        }
        self.state().menu = Some(menu.clone());
        self.draw(menu);
        if chosen.checked {
            return;
        }
        let client = self.state().client.clone();
        let Some(client) = client else { return };
        let shell = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            let Err(failure) = client.post("/api/exit", &chosen.choice).await else {
                return;
            };
            let (redraw, now) = {
                let mut state = shell.state();
                let menu = state.reach.as_ref().map(model::menu);
                state.menu.clone_from(&menu);
                (menu, now_ms())
            };
            if let Some(menu) = redraw {
                shell.draw(menu);
            }
            let message = match &chosen.name {
                Some(exit) => copy::exit_switch_failed(exit, current.as_deref()),
                None => copy::internal(&failure.message),
            };
            shell.notify(Notice {
                tag: format!("exit-failed:{now}"),
                title: copy::APP.to_owned(),
                body: Some(message),
                target: "/".to_owned(),
            });
        });
    }

    /// Opens a `kith://join/` link in the page's join flow.
    pub(crate) fn deep_link(self: &Arc<Self>, link: &str) {
        if let Some(code) = join_code(link) {
            self.open(Some("/".to_owned()), Some(code));
        }
    }

    /// Shows the window, opening it if needed. `path` moves it to that page; `None`
    /// leaves an open window where it is. `join` hands an invite code to the page, which
    /// only a fresh page reads, so it reopens the window.
    pub(crate) fn open(self: &Arc<Self>, path: Option<String>, join: Option<String>) {
        if let Some(window) = self.window() {
            if join.is_none() {
                let port = self.state().window.as_ref().map(|open| open.page.port);
                if let (Some(path), Some(port)) = (path, port)
                    && let Ok(url) = tauri::Url::parse(&page::address(port, &path))
                {
                    let _not_moved = window.navigate(url);
                }
                reveal(&window);
                return;
            }
            let _already_gone = window.destroy();
            self.state().window = None;
        }
        let generation = {
            let mut state = self.state();
            if state.opening {
                return;
            }
            state.opening = true;
            state.opened = state.opened.saturating_add(1);
            state.opened
        };
        let shell = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            let path = path.unwrap_or_else(|| "/".to_owned());
            let opened = shell.build_window(generation, &path, join.as_deref()).await;
            let mut state = shell.state();
            state.opening = false;
            if let Some(open) = opened {
                state.window = Some(open);
            }
        });
    }

    async fn build_window(
        self: &Arc<Self>,
        generation: u64,
        path: &str,
        join: Option<&str>,
    ) -> Option<Open> {
        let bound = Bound::new(self.source()).await.ok()?;
        let url = tauri::Url::parse(&bound.link(path, join)).ok()?;
        let page = bound.serve();
        let port = page.port;
        let label = format!("main-{generation}");
        let shell = Arc::clone(self);
        let window = WebviewWindowBuilder::new(&self.app, &label, WebviewUrl::External(url))
            .title(copy::APP)
            .inner_size(WINDOW_SIZE.0, WINDOW_SIZE.1)
            .min_inner_size(WINDOW_MIN.0, WINDOW_MIN.1)
            .visible(false)
            .on_navigation(move |url| {
                if page::stays_inside(url, port) {
                    return true;
                }
                shell.leave(url);
                false
            })
            .on_page_load(|window, load| {
                if load.event() == PageLoadEvent::Finished && !window.is_visible().unwrap_or(true) {
                    reveal(&window);
                }
            })
            .build()
            .ok()?;
        let background = match window.theme() {
            Ok(Theme::Dark) => BG_DARK,
            Ok(_) | Err(_) => BG_LIGHT,
        };
        let _not_painted = window.set_background_color(Some(color(background)));
        let shell = Arc::clone(self);
        window.on_window_event(move |event| {
            if matches!(event, WindowEvent::Destroyed) {
                shell.closed(generation);
            }
        });
        #[cfg(target_os = "macos")]
        let _no_dock = self
            .app
            .set_activation_policy(tauri::ActivationPolicy::Regular);
        let fallback = window;
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(WINDOW_SHOW_FALLBACK).await;
            if !fallback.is_visible().unwrap_or(true) {
                reveal(&fallback);
            }
        });
        Some(Open { label, page })
    }

    /// A link the page tried to follow that is not the page: web links open in the
    /// browser, invite links in the join flow, anything else nowhere.
    fn leave(self: &Arc<Self>, url: &tauri::Url) {
        match url.scheme() {
            "http" | "https" => {
                let link = url.to_string();
                std::thread::spawn(move || {
                    let _no_browser = opener::open(link);
                });
            }
            "kith" => self.deep_link(url.as_str()),
            _ => {}
        }
    }

    /// The window `generation` was destroyed: its page server stops with it.
    fn closed(&self, generation: u64) {
        let label = format!("main-{generation}");
        let mut state = self.state();
        if state
            .window
            .as_ref()
            .is_some_and(|open| open.label == label)
        {
            state.window = None;
        }
        drop(state);
        #[cfg(target_os = "macos")]
        let _no_dock = self
            .app
            .set_activation_policy(tauri::ActivationPolicy::Accessory);
    }
}

fn color(rgb: Rgb) -> Color {
    Color(rgb.0, rgb.1, rgb.2, u8::MAX)
}

fn reveal(window: &WebviewWindow) {
    let _shown = window.show();
    let _restored = window.unminimize();
    let _focused = window.set_focus();
}

fn draw_tray(app: &AppHandle, tray: &TrayIcon, menu: &Menu) -> tauri::Result<()> {
    let native = NativeMenu::new(app)?;
    for line in &menu.status {
        native.append(&MenuItem::with_id(app, "status", line, true, None::<&str>)?)?;
    }
    if !menu.status.is_empty() {
        native.append(&PredefinedMenuItem::separator(app)?)?;
    }
    for (index, exit) in menu.exits.iter().enumerate() {
        let id = format!("exit:{index}");
        native.append(&CheckMenuItem::with_id(
            app,
            id,
            &exit.label,
            true,
            exit.checked,
            None::<&str>,
        )?)?;
    }
    if !menu.exits.is_empty() {
        native.append(&PredefinedMenuItem::separator(app)?)?;
    }
    if let Some(needs) = &menu.needs {
        native.append(&MenuItem::with_id(
            app,
            "needs",
            &needs.label,
            true,
            None::<&str>,
        )?)?;
    }
    native.append(&MenuItem::with_id(
        app,
        "open",
        copy::OPEN,
        true,
        None::<&str>,
    )?)?;
    native.append(&PredefinedMenuItem::separator(app)?)?;
    native.append(&MenuItem::with_id(
        app,
        "quit",
        copy::QUIT,
        true,
        None::<&str>,
    )?)?;
    tray.set_menu(Some(native))?;
    tray.set_icon(Some(tray_icon(menu.icon)))?;
    #[cfg(target_os = "macos")]
    tray.set_icon_as_template(true)?;
    Ok(())
}

fn tray_icon(state: IconState) -> Image<'static> {
    let paint = if cfg!(target_os = "macos") {
        icon::TEMPLATE
    } else {
        icon::PANEL
    };
    Image::new_owned(icon::render(state, ICON_SIZE, paint), ICON_SIZE, ICON_SIZE)
}

/// Sets up the tray, the deep links and the watcher once Tauri is ready.
pub(crate) fn setup(app: &mut tauri::App, origin: Origin) -> tauri::Result<()> {
    use tauri_plugin_deep_link::DeepLinkExt as _;

    #[cfg(target_os = "macos")]
    app.set_activation_policy(tauri::ActivationPolicy::Accessory);
    let fixture = matches!(origin, Origin::Fixture(_));
    let shell = Arc::new(Shell {
        app: app.handle().clone(),
        origin,
        state: Mutex::new(State::default()),
    });
    app.manage(Arc::clone(&shell));

    let starting = Menu {
        icon: IconState::Off,
        status: Vec::new(),
        home: "/".to_owned(),
        exits: Vec::new(),
        needs: None,
    };
    let tray = TrayIconBuilder::with_id(TRAY)
        .tooltip(copy::APP)
        .icon(tray_icon(starting.icon))
        .icon_as_template(cfg!(target_os = "macos"))
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| {
            if let Some(shell) = app.try_state::<Arc<Shell>>() {
                shell.on_menu(event.id().as_ref());
            }
        })
        .build(app)?;
    draw_tray(app.handle(), &tray, &starting)?;

    let links = app.deep_link();
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    let _not_registered = links.register_all();
    let on_link = Arc::clone(&shell);
    links.on_open_url(move |event| {
        for url in event.urls() {
            on_link.deep_link(url.as_str());
        }
    });
    if let Ok(Some(urls)) = links.get_current() {
        for url in urls {
            shell.deep_link(url.as_str());
        }
    }

    tauri::async_runtime::spawn(crate::watch::run(Arc::clone(&shell)));
    if !fixture {
        crate::autostart::offer_once(app.handle());
    }
    Ok(())
}

/// A second launch: deep links reach [`Shell::deep_link`] through the deep-link plugin;
/// a plain launch shows the window.
pub(crate) fn second_launch(app: &AppHandle, argv: &[String]) {
    if argv.iter().any(|arg| arg.starts_with("kith://")) {
        return;
    }
    if let Some(shell) = app.try_state::<Arc<Shell>>() {
        shell.open(None, None);
    }
}

/// Keeps the shell in the tray when its window closes, and shows the window when the app
/// is opened again.
pub(crate) fn on_event(app: &AppHandle, event: RunEvent) {
    match event {
        RunEvent::ExitRequested {
            code: None, api, ..
        } => api.prevent_exit(),
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => {
            if let Some(shell) = app.try_state::<Arc<Shell>>() {
                shell.open(None, None);
            }
        }
        _ => {}
    }
}

/// Milliseconds since the Unix epoch, by this machine's clock.
#[expect(
    clippy::disallowed_methods,
    reason = "the shell is a driver: notification windows are timed by the wall clock"
)]
pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}
