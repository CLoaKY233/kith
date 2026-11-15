# src/ui

| Component | Use it for |
|---|---|
| `Page` | Every screen: title, one-line description, header actions |
| `Section` | A titled block inside a page |
| `Panel`, `PanelGrid` | Hairline box with label + right slot; grid of panels sharing hairlines (`span` 3–12) |
| `Stat` | One labelled big number with unit and secondary line |
| `Button` | Every button: primary, secondary, ghost, danger; sm/md; icon; busy |
| `Badge` | Short mono tag: neutral, strong, danger |
| `StatusDot` | Online / offline / problem square |
| `Empty` | Nothing to show yet, with an optional action |
| `Copy` | Mono value that copies on click |
| `toast()`, `Toaster` | Notifications; mount `Toaster` once at the root |
| `Skeleton` | Reserved space while data loads |
| `TimeSeries` | Any metric over time; `useCrosshair` syncs hover across panels |
| `Spark` | Tiny trend inside a table row |
| `BarList` | Ranked values (busiest devices, drops by reason) |
| `Meter` | One value against a known maximum |
| `fmt` | bytes, rate, ms, percent, count, clock, relative, absolute |
