# kith-app

The desktop shell (design spec §15, M2): kith in the menu bar.

- **Tray menu:** `● Connected to home · 11 of 12 devices`, the exit choices, `1 needs you…`, `Open kith`, `Quit`.
- **Icon:** solid when kith works, a dot when something needs you, hollow when kith is off. Drawn in `src/icon.rs`.
- **Notifications:** join requests (admins), the exit going offline, a recently used device going offline, the daemon stopped for 10 s. The only action is `Review`.
- **Autostart:** offered once, on the first launch.
- **Links:** `kith://join/<code>` opens the page's join flow.
- **Window:** the kith page, served by `kith-ui-core` on a loopback port while the window is open. Closing the window destroys the web view and stops that server.

The shell reads kith only through `kith-ui-core`, in process for the tray and over loopback for the window, so it shows what the page shows. The web view has no `invoke` access.

## Build and run

The app is its own Cargo workspace, so the root workspace never builds Tauri.

```sh
cargo run -- --fixture ../../crates/management/kith-ui/web/fixtures/office.json
cargo test
npm ci && npx tauri build --bundles app   # or dmg, deb, appimage
```

`--fixture` shows a recorded network and needs no daemon. Build the page first (`npm ci && npm run build` in `crates/management/kith-ui/web`); without it the window shows `fallback.html`.

Linux needs `libwebkit2gtk-4.1-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev` and `libxdo-dev`.

## What the page receives

The window's first address is `http://127.0.0.1:<port><path>#t=<token>`, as `kith ui` opens. A `kith://join/<code>` link adds `&join=<code>` to that fragment.
