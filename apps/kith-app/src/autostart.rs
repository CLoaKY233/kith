//! Starting at login, offered once (design spec §15 M2): the first launch asks, and the
//! answer is never asked for again. A marker file in the app's config folder remembers
//! that the question was asked.

use tauri::{AppHandle, Manager as _};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_dialog::{DialogExt as _, MessageDialogButtons};

use crate::copy;

/// The marker's file name.
const MARKER: &str = "autostart-offered";

/// Asks once whether to start at login, unless it already does.
pub(crate) fn offer_once(app: &AppHandle) {
    let Ok(dir) = app.path().app_config_dir() else {
        return;
    };
    let marker = dir.join(MARKER);
    if marker.exists() {
        return;
    }
    let remembered = std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&marker, b""));
    if remembered.is_err() || app.autolaunch().is_enabled().unwrap_or(false) {
        return;
    }
    let handle = app.clone();
    app.dialog()
        .message(copy::AUTOSTART_QUESTION)
        .title(copy::APP)
        .buttons(MessageDialogButtons::OkCancelCustom(
            copy::AUTOSTART_YES.to_owned(),
            copy::AUTOSTART_NO.to_owned(),
        ))
        .show(move |yes| {
            if yes {
                let _not_enabled = handle.autolaunch().enable();
            }
        });
}
