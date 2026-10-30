# The macOS and iOS port

How the native builds are put together, and what is known to be left. The build steps are in the
[README](../README.md#macos-and-ios).

## Mac

`scripts/macos-app.sh` (`make app`) builds the player binary (no `dev` feature), wraps it in
`dist/Azeroth Survivors.app` with a small C launcher that points `WOW_DATA` at
`Contents/Resources/Data` and `BENILLA_HOME` at `~/Library/Application Support/AzerothSurvivors`,
clones the data in (`cp -c`, an APFS clone) and signs the bundle ad hoc. `APP_NO_DATA=1` leaves the
data out.

## iOS

### Build

- **The game is a static library.** `crates/azeroth-survivors-ios` exports
  `azeroth_survivors_main`, which `ios/App/main.m` calls on the main thread (winit's UIKit event
  loop must own it). It defaults the render scale to 0.6 and the state folder to the app's
  `Documents/`, then runs the survivors mode.
- **`ios/build-rust.sh`** is the Xcode project's first build phase. It builds the release profile
  for the device or the simulator into `target/ios`, in a fixed environment (`env -i`): Xcode's
  settings vary between runs, and one variable a build script reads (`SDKROOT`, through
  `tracing-oslog`) was enough to rebuild all of Bevy. A changed deployment target cleans the
  target's artifacts once, so C objects never carry another minimum iOS. Lua is linked as its own
  `liblua5.1.a`, which the staticlib leaves out. A device build also copies the data into the
  bundle: `wow-data/Trimmed` when it exists, else the full `wow-data/Data`, or `WOW_DATA_SRC`.
- **`ios/patch-sdk-version.sh`** is the last phase: it stamps the binary as built against the
  iOS 18.0 SDK. winit 0.30 has no UIScene lifecycle, which apps built with the iOS 27 SDK must
  adopt or fail to launch.
- **Signing** lives in `ios/Signing.xcconfig`, which includes the git-ignored `ios/Local.xcconfig`
  for your team.

### Play on a touch screen

`crates/benilla-app/src/survivors/touch.rs`:

- **Taps are mouse clicks.** A touch moves the cursor and presses the left button, released a
  frame later so a button sees the press. The rest of the UI needs no touch code.
- **Twin sticks.** A finger landing on the left 45% of the screen (off a button) is the move
  stick; one on the right is the aim stick, which turns the hero and aims the mobility spell.
  The aim point is projected from the stick onto the ground, turned with the camera when it is
  unlocked.
- **On-screen keys.** The HUD's mobility slot grows to thumb size at the bottom centre and presses
  the mobility key; the pause button presses Esc. Both are hit-tested before the sticks.
- **Safe area.** The insets come from winit (inner against outer window frame) and become an
  invisible border on every full-screen UI root, in the background's colour so dimmers still
  reach the edges. The pause button sits past the right inset, centred between the minimap and
  the screen edge, above where a landscape Dynamic Island can be.

### Phone specifics

- **Experience gems** are capped at 64 on the ground; the excess merges into the nearest one.
  Each gem is about 50 entities, and long runs used to slow down under thousands of them.
- **Backgrounding** (`AppLifecycle::WillSuspend`, an occluded window) pauses the run.
- **Thread priorities.** iOS flags a user-interactive thread that waits on a lower class. The
  task pools, the audio render and device-watch threads run user-interactive on iOS
  (`crates/benilla-world/src/boot.rs`, `thread_qos.rs`, `sound/output/cpal.rs`); the desktop
  keeps its classes.
- **Logging.** tracing-oslog turns every span into a log entry; the iOS log filter keeps the
  noisy targets (winit's app state, bevy_render, the UI script extractor) at warnings.

## Trimmed data

The full 1.12.1 client is 5 GB; the mode reads under a gigabyte of it.
`scripts/trim-data.sh` builds `wow-data/Trimmed` (about 1 GB):

1. **Census.** The native build runs with `WOW_DATA_TRACE`, which records every name the patch
   chain resolves (`crates/benilla-formats/src/chain.rs`), under the census autopilot
   (`WOW_SURVIVORS_AUTO=<class>:census` and `:census-run`, `survivors/auto.rs`): one run tours
   every class and look at the menu, and one per battleground casts the whole spell pool, spawns
   every enemy and boss of the zone and uses the movement spell.
2. **Pack.** `mpqpack` writes one `patch.MPQ` with `benilla_mpq::ArchiveWriter` (V1 archive, 4 KiB
   zlib sectors, encrypted tables, a listfile). Beyond the traced names it closes over what a run
   could still reach: every terrain tile in each arena's reach at the widest view distance with its
   minimap art, every texture, model and WMO those files name, every variation of a sound kit and
   every kit in a creature's or zone music's folder, creature voices, all races' combat sounds,
   weapon sounds, room tones, and blood art. The small archives (interface, dbc, fonts, misc, base,
   backup, patch-2) stay whole, and every name of theirs that a dropped archive overrode or
   deleted is packed too, so nothing falls back to a stale copy.
3. **Check.** `scripts/trim-data.sh check` runs the census again on the trimmed set with other
   classes and lists what it read that the trimmed set lacks. `scripts/ios-sim-check.sh` plays all
   sixteen zones in a booted simulator on a data folder and counts files not found.

## Known limits

- **Textures are not compressed on iOS.** wgpu reports BC texture compression only on macOS, so
  the textures are decoded to RGBA8 and take four to eight times the memory (about 2.2 GB in a
  run on an iPhone 15 Pro Max). ASTC transcoding or a wgpu patch would bring that down.
- **A "Hang Risk" warning at launch.** Xcode's Thread Performance Checker sometimes reports one
  wait on Bevy's asset server lock during the first frame. Every thread of the game is
  user-interactive by then; the waiting is on a std `RwLock`, whose holder the checker cannot see.
- **The UIScene stamp** is a stopgap until winit adopts the scene lifecycle.
