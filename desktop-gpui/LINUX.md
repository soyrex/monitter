# Build on Omarchy / Arch Linux

This branch is a personal development checkpoint for the GPUI desktop and its
independent Rust backend. The instructions target x86_64 Omarchy. Native Linux
compilation/runtime is not yet verified; macOS is the tested host.

## Dependencies

Install the build tools and the GTK/WebKit dependencies required by the existing
Rust backend and webview crates. Omarchy already supplies a Wayland compositor;
retain its existing GPU driver configuration.

```sh
sudo pacman -S --needed base-devel clang cmake pkgconf git nodejs npm \
  gtk3 webkit2gtk-4.1 libappindicator fontconfig freetype2 openssl \
  libxkbcommon-x11 libx11 libxcb wayland vulkan-headers vulkan-icd-loader
```

Rust 1.95 or newer is required; this source has been checked with Rust 1.98.1.
Use your existing Rust installation if it meets that minimum. With an existing
rustup installation, select the tested version in the checkout:

```sh
rustup toolchain install 1.98.1
rustup override set 1.98.1
```

## Clone, build and run

```sh
git clone --branch experiment/gpui-desktop --single-branch \
  https://github.com/soyrex/monitter.git monitter-gpui
cd monitter-gpui
npm ci
npm run build
npm run build:gpui -- --jobs 2
npm run dev:gpui -- --jobs 2
```

Run the optional rustup commands above after entering `monitter-gpui`, before
building. The web build supplies assets required by the shared backend's Tauri
context. The GUI itself is native GPUI. Two compiler jobs limit peak memory use
on the laptop; use `--jobs 1` if the first compile exhausts memory. Keep at least
10 GiB free for a cold build, with additional room for downloaded dependencies.

Debug executables are `.cargo-target/gpui/debug/monitter-gpui-desktop` and
`.cargo-target/gpui/debug/monitter-gpui-server`. The GUI starts its sibling backend
when needed. Closing the GUI leaves that backend running. To run the backend in a
terminal for diagnostics, start it before the GUI:

```sh
.cargo-target/gpui/debug/monitter-gpui-server
```

Ctrl+C gracefully shuts down that foreground server. `dev:gpui` writes GUI output
under `artifacts/gpui/debug/preview.stdout.log` and `preview.stderr.log`. For a
foreground GUI run, execute `.cargo-target/gpui/debug/monitter-gpui-desktop` directly.

## Wayland and current limits

GPUI chooses Wayland when `WAYLAND_DISPLAY` is set, which is normal on Omarchy.
If a renderer/window issue requires trying XWayland, preserve `DISPLAY` and run:

```sh
env -u WAYLAND_DISPLAY -u ZED_HEADLESS \
  .cargo-target/gpui/debug/monitter-gpui-desktop
```

The embedded browser and native microphone capture explicitly report unsupported
outside macOS. XWayland does not enable those features. Hyprland wallpaper sampling
and several macOS-native visual effects also remain unavailable. CLI harnesses and
their authentication must be available locally on this Linux machine; no Mac
credentials, sessions or application profile are copied by these instructions.

Dependency references: [Omarchy manual](https://omarchy.org/manual/),
[Wry 0.55.1 Linux requirements](https://docs.rs/wry/0.55.1/wry/#linux),
[Arch WebKitGTK](https://archlinux.org/packages/extra/x86_64/webkit2gtk-4.1/),
[Arch AppIndicator](https://archlinux.org/packages/extra/x86_64/libappindicator/).
