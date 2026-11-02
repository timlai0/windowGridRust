# ADR 006: iced UI fork (`wg-app-iced`)

**Date:** 2026-11-01  
**Status:** Accepted (opt-in fork; `wg-app` with Slint remains the default)

## Context

[ADR 004](004-rust-app.md) chose Slint for the app's windows. Slint's layout lives in a DSL file (`ui/app.slint`, compiled by `build.rs`) and the Rust side reaches it through string-named properties and callbacks. To test the app as plain Rust end to end, the same windows were rebuilt on [iced](https://docs.rs/iced/latest/iced/) (0.14), a pure-Rust GUI library, without changing anything else about the app.

## Decisions

**A separate crate beside `wg-app`, not a replacement.** `wg-app-iced` builds `WindowGridRust-iced.exe` (`build-app-iced.ps1`). It is a workspace member but not a default member, so `cargo build` and `cargo test` in `rust/` do not compile iced; name it with `-p wg-app-iced`.

**Reuse by a feature gate, not a copy.** `wg-app` gets a default `slint-ui` feature that guards the `slint` dependency, the Slint compile step in `build.rs`, the `ui` and `app` modules, the `wg-app` binary and its two Slint tests. `wg-app-iced` depends on `wg-app` with `default-features = false`, so it gets `viewmodel` (the settings model with its clamps, presets and summaries), `config`, `installer`, `supervisor`, `tray`, `shared`, `components`, `dialogs`, `osopts` and `args` exactly as they are, and no Slint is compiled. Only the windows and the event loop are new: about 3,000 lines of Rust (about 400 of them unit tests) where the Slint app has 1,750 (the 982-line `.slint` file plus `ui.rs` and `app.rs`). Declaring the layout in Rust is wordier than the DSL, and this fork also does more (the layout adapts to the window size, below); in exchange the update logic and the layout rules are plain functions that the unit tests call directly.

**One long-lived `iced::daemon`.** A daemon opens no window by default and does not stop when its windows close; it ends only through `iced::exit()`. That is the shape of a tray app. The settings window, the setup dialog and the splash screen are daemon windows opened on demand. All the startup branches (`/install`, first-run setup, normal start) run inside the one daemon, because an event loop cannot be created twice in a process. `App::enter_tray_mode` is the former `run_tray_app`; it also runs after a first-run setup that did not hand over to an installed copy.

**The tray stays Win32, bridged by a channel.** `wg_app::tray` is unchanged: a hidden window whose messages the thread's event loop dispatches. iced's winit loop does that for every window of the thread, so the tray callback only pushes the event into a futures channel, and a `Subscription` turns it into a message. The subscription exists only while the tray app is up.

**Software renderer.** iced is built with `default-features = false` and `tiny-skia` (plus `thread-pool` and `canvas`), the equivalent of Slint's `renderer-software`: no GPU or web runtime. Fonts are the system's (Segoe UI).

**A look built on the grid.** The app is about a grid, so the windows are drawn on one. Everything sits on an 8 px unit with square (2 px) corners and hairline borders. A faint grid, every fourth line a little stronger, is drawn behind each page (`backdrop.rs`) and stays put while the page scrolls over it. Cards are cells with a heading strip; settings are tables whose rows are a label cell and a control cell (`widgets::table`, `setting_row`); pages have coordinates (`01 / CONFIGURE`) that the navigation repeats; and the current page is outlined and tinted like the overlay's selection rectangle. The colours come from a handful of tokens for the light and dark mode (`style::Tokens`).

**Any window size.** WindowGridRust snaps windows, including its own settings window, into grid cells, and a snap bypasses a window's minimum size (verified: `SetWindowPos` takes any size). So the settings window cannot assume its default size; its minimum is a nominal 320 x 380 and `settings::Layout::for_width` turns the actual width into the arrangement:

| Window width | Navigation | Cards | Table rows |
|---|---|---|---|
| 720 and up | full panel (224 px) | side by side once the content is 560 px wide, else stacked | control beside the label when the card is 380 px (260 px for switch-only tables) |
| under 720 | rail of coordinates (56 px) | stacked | control under the label when narrow |

Below 440 px of content the margin and the grid halve to 16 px; the button bar wraps; a shortcut row becomes two lines. `Layout` is a pure function with unit tests. The setup dialog is fixed-size, but its body scrolls so it survives being snapped small.

**Questions are drawn in the window.** The exit and reset confirmations of the settings window, and the setup dialog's "cancel installation?", are a `stack` overlay (dimmed layer, Escape cancels, Enter confirms), instead of a nested native message box inside an event handler.

## Deviations from the Slint app (intentional)

- **Cancel discards.** The Slint settings window was hidden, not closed, so unsaved edits reappeared the next time it opened. Here Cancel closes the window and the next open reads the saved settings.
- **Radial gradient.** iced's canvas draws linear gradients only. The radial fill of the preview is 24 concentric ellipses from the darker centre to the start colour. `Frame::with_clip` does not clip in the software renderer, so the ellipses are sampled and clamped into the selection rectangle in code (`preview::ring`, unit-tested).
- **Drop-downs.** `ComboBox` became `pick_list`; the key list is the same several hundred legacy names.
- **About.** The Slint license requires showing `AboutSlint`; this build has no Slint, so the card says it is built with iced (MIT).
- **Presets** wrap onto further lines instead of overflowing, whatever the width.
- **Look and size.** The Fluent-style look of the Slint windows is replaced by the grid look above, and the settings window works at any size where the Slint one has a fixed 800 x 520 minimum that a snap can bypass. The Slint app was not changed.

## Same identity as `wg-app`

The fork uses the same settings file, log, mutex, shared-memory and window-message names, so only one of the two runs at a time: starting one replaces the other, which is also how switching between them is done. `/install` copies whichever executable ran, under its own file name.

## Results

Release builds, isolated names, one run each, measured 8 seconds after start (working set / private bytes of the app process alone):

| | `wg-app` (Slint) | `wg-app-iced` |
|---|---|---|
| Executable (includes overlay and hooks) | 10.55 MB | 3.59 MB |
| Idle in the tray | 27.7 MB / 2.9 MB | 19.9 MB / 3.4 MB |
| Settings window open | 33.9 MB / 6.9 MB | 27.7 MB / 9.1 MB |

The iced build is much smaller on disk and has the smaller working set, but its private memory is higher (software-rendered frame buffers and glyph caches), so neither is better on every axis.

## Responsiveness

The first version of the grid look felt laggy: hovering over the navigation took 83-199 ms to paint in a headless model of the window (the layout, drawing, damage tracking and rasterizing stages run in a test process) and 67-97 ms in the real window (a figure that includes about 30 ms for the capture used to measure it). iced's software renderer repaints only what changed since the last frame, and three things in the UI defeated that:

1. **Canvases drawn fresh every frame** (the page grid, the preview). iced can only treat a canvas as unchanged when it is handed *the same cached geometry* (compared by `Arc` identity); anything else counts as changed every frame, so a hover anywhere repainted the whole canvas area through ~100 separate rectangles. Both canvases now draw through a `canvas::Cache` that remembers what it was drawn for (`memo.rs`).
2. **Idle navigation cells drew no quad.** A button without a background draws nothing, and iced compares two frames' quads by *position in the list*, so the hover quad appearing shifted every later quad and damaged the whole navigation column. Idle cells now have a transparent background, so the quad is always there (`style::nav_cell`).
3. **The page colour was a window-sized container background.** That is one huge quad, redrawn through a clip mask for *every* damaged rectangle, so each small rectangle cost nearly a full-window repaint. The page colour is now the window's background colour (which iced fills with a fast unmasked fill); the splash window, which must stay transparent, has its own clear theme.

A page switch changes everything, which iced then repaints as dozens of separate rectangles (43 of them, ~190 ms). When a layer's bounds differ between two frames, iced instead damages the whole layer as one rectangle, so the container around the stack that holds the page is a hundredth of a pixel shorter for each step down the navigation (`settings::content`): invisible, and a page switch becomes one repaint.

| Headless, 960 x 680, release | Before | After |
|---|---|---|
| Hover over a navigation cell (worst move) | 83-199 ms (original code) | 1.0-1.6 ms |
| Switch pages | 189 ms, 43 rectangles (measured with fixes 1 and 2 already in) | 14 ms, 3 rectangles |
| Stepper or preset button | 24 ms (same) | ~10 ms |
| Sliders, toggles | not measured before | 1-6 ms |
| A whole frame, for reference | | 10-15 ms |

In the real window, with each figure including the ~30 ms capture: hover latency 67-97 ms with all three defects restored, 31-50 ms with the fixes.

### The build profile was the next bottleneck

With the repaints minimised, what was left was the cost of painting pixels, and the workspace's release profile (`opt-level = "s"`, there for the hook DLLs and the overlay) made the CPU renderer 2.5-3 times slower than it has to be. Building every crate at `opt-level = 3` showed the ceiling; naming only the crates that do the drawing got the same result (`[profile.release.package.*]` in `rust/Cargo.toml`):

| Headless, release | `s` everywhere | `3` for the drawing crates |
|---|---|---|
| Whole window, 640 x 480 | 8.7 ms | 3.0 ms |
| Whole window, 960 x 680 | 12.5 ms | 5.5 ms |
| Whole window, 1920 x 1080 (maximised) | 19.8 ms | 7.2 ms |
| Whole window, 2560 x 1440 | 28.1 ms | 9.8 ms |
| Stepper or preset button | 9.7 ms | 3.5-4 ms |
| Slider: overlay opacity | 9.5 ms | 2.0 ms |
| Switch pages | 11.3-12.3 ms | ~7.5 ms |

(`resize_report` and `interaction_report` in `tests/frame_cost.rs`; medians of 9 repaints for the sizes, worst of a sweep for the interactions; single runs on a machine with other load, so expect a few tenths of a millisecond of noise.) Naming only the three tiny-skia crates was not enough (the opacity slider stayed at 5 ms); the rest of the gain is in iced's own crates, cosmic-text and the font readers. The Slint app has its own copies of some of these (a second tiny-skia, other skrifa and read-fonts), so the entries carry versions and the Slint app's build is unchanged; `swash`, `zeno` and the `lyon` crates are shared with Slint and were left alone (leaving them out cost nothing measurable). `WindowGridRust-iced.exe` grows from 3.8 to 4.8 MB. Cargo warns when an entry stops matching a package after a dependency update, which is the cue to update the list.

### Checked and fine

- **Startup.** From starting the process to the tray window: ~160 ms. From a double-click on the tray icon to the settings window: ~60 ms, and to its first painted frame 170-270 ms (three runs, each including 30-40 ms polling granularity). A cold first frame spends 24 ms loading the system fonts and 11 ms in layout and drawing; the rest is the operating system creating the window. Loading the fonts in advance would save at most 24 ms, so it is not done.
- **Idle.** With the settings window open and untouched: 0 ms of CPU time in 8 s (nothing polls, redraws on a timer or wakes up), 10 threads, 21 MB working set.
- **Resizing the real window** (`resizing_the_window_report` in `lifecycle`): 1280 x 800 up to 2560 x 1400 show the new size in 32-60 ms including the capture used to detect it, so reallocating the window's surface and presenting it add no visible stall.
- **A call that could block the interface.** "Show window contents while dragging" calls `SystemParametersInfo` with the broadcast flag, which makes Windows tell every open program and wait for them, so a busy program delays it. It used to run on the interface thread (as it does in the Slint app). It now runs on its own thread: the switch moves at once, a second press is ignored until Windows has answered, and the switch then shows what Windows says. This was not timed, because timing it means changing a system setting; it is the documented behaviour of that call.

`tests/frame_cost.rs` guards this without clocks, so it does not flake: a hover must damage less than 5% of the window in at most four rectangles, moves inside one cell must repaint nothing, and switching between *every pair* of pages must be at most four rectangles. Its timing reports (`--ignored --nocapture`) are for finding the next problem.

## Verified

- Unit tests (`cargo test -p wg-app-iced`): update logic of the settings window and the setup dialog (grid clamps and presets, shortcut rows, save, reload, reset and exit confirmations, theme fallback), preview geometry, and construction of every view.
- `tests/lifecycle.rs` runs the real isolated executable: components extracted, overlay and hook hosts started and published, clean quit that takes the helpers with it, a double-click message posted to the app's own tray window opening the settings window (the tray-to-iced bridge), and PNG captures of the real windows in `target\tmp\iced-*.png`: every settings page at its default size and resized to 816 x 560, 620 x 520 and 380 x 700 (the window really takes those sizes), the confirmation, and the setup dialog. Each page is also checked for *card-coloured pixels in the page area*. `tests/frame_cost.rs` is headless (see Responsiveness). `lifecycle` also has an opt-in test that moves the real pointer over the navigation and times the repaint. The captures were reviewed as images; that is how the unclipped radial preview, the shadow artifact, and a page that rendered completely empty (below) were found.
- The splash screen was run once on the real desktop: a centred 256 x 256 always-on-top window that closes by itself, with the desktop showing through around the icon (real transparency).
- `wg-app` itself still passes its 47 unit tests and the Slint render test with the feature gate in place, and `cargo clippy` is clean for both crates.

## Renderer and layout limitations that shaped the code

The first two come from iced's software renderer, the third from its layout; none comes from this app, and all are worked around rather than fixed:

- `Frame::with_clip` neither clips nor translates, so the preview clips its gradient itself.
- A quad's shadow is painted without the clip mask and darkens the contents of the box on partial redraws, so nothing here has a shadow; the confirmation box has a border.
- A container takes on the `Fill` height of anything inside it. A table row has a `Fill`-height divider, so two tables side by side in a row inside the scrolling page were given an unbounded height and the page rendered **empty**. Rows that hold cards are pinned to `Shrink` height (`Layout::split`). A page that lays out to nothing still shows the navigation, the grid and the buttons, so the test counts card pixels in the page area instead of just checking that the window is not blank.

Also: an iced button puts its content at the top, so the navigation cells wrap their content in a container that fills and centres it.

## Open items

- Elevated installs, the all-users Start Menu shortcut and the logon task are as untested here as in `wg-app` (they need administrator rights). The logic is shared, but in the real window the setup dialog was only opened; accepting and cancelling it are covered by unit tests, not by clicking.
- The tray *menu* (right-click: Settings, Install, Exit) is not driven; only the double-click event is. Exit shares `App::quit` with the timed quit that the lifecycle test does exercise.
- `wg-app`'s own isolated lifecycle suite was not re-run after the feature gate; the gate changes which modules compile, not what they do.
- Keyboard focus order and screen-reader support of iced widgets were not reviewed; iced has no accessibility tree yet.
- The measurements are from one machine at a 960 x 680 window and 100% display scaling. Rasterizing scales with the window's area and the scale factor, so a large window on a high-DPI display costs proportionally more (a whole frame is about 5 ms here at 960 x 680 and about 10 ms at 2560 x 1440, see the build profile above).
- The page-switch trick and the requirement that canvases come from a cache rest on how iced 0.14's software renderer compares frames. If an iced update changes that, `switching_pages_is_one_repaint_not_dozens` and `a_hover_repaints_only_the_cells_that_changed` fail, which is what they are for.
- Steppers and the preset buttons still repaint about half the window (the preview changes with them), 3.5-4 ms now. The text shaper (`harfrust`) is still built for size; shaping shows up as 0.2-0.5 ms of layout per change (2 ms on the shortcuts page) and was not worth another entry.
- Verified on one machine (Windows 11, x64, 100% display scaling, dark mode). Other DPI settings, Windows versions, ARM64 and the light theme were not looked at; the light colours are defined (`style::Tokens`) but never rendered in a review.
- The behavior of the drop-down list (open state, scrolling the several hundred key names) was not captured; only its closed look was reviewed.
- The setup dialog is not responsive beyond scrolling its body; the grid look and the layout rules were reviewed on the settings window.
