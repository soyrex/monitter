//! Retained native Vim command line and bounded workspace command parser.
use crate::design::ui_scaled_px as px;
use gpui_kit::{
    component::input::{Input, InputEvent, InputState},
    prelude::*,
    *,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VimTabTarget {
    Index(usize),
    Relative(isize),
    Last,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VimCommand {
    NewChat,
    SelectTab(VimTabTarget),
    CloseTab(Option<VimTabTarget>),
    OnlyTabs,
    MoveTab(String),
    Split {
        direction: String,
        size: Option<u32>,
    },
    Terminal,
    ClosePane(Option<usize>),
    OnlyPane,
    FocusPane(String),
    Rotate(i8),
    Exchange(Option<usize>),
    MovePane(String),
    Equalize,
    Resize {
        axis: String,
        amount: Option<isize>,
        maximize: bool,
    },
    Help,
}

#[derive(Clone, Debug)]
pub enum VimCommandBarEvent {
    Run(VimCommand),
    Dismiss,
}

#[derive(Default)]
struct VimCommandRegistry {
    bars: std::collections::HashMap<WindowId, Entity<VimCommandBar>>,
}
impl Global for VimCommandRegistry {}

pub struct VimCommandBar {
    input: Entity<InputState>,
    open: bool,
    error: Option<String>,
    help: bool,
    palette: Option<crate::design::Palette>,
}
impl EventEmitter<VimCommandBarEvent> for VimCommandBar {}

impl VimCommandBar {
    pub fn new(window: &mut Window, cx: &mut Context<crate::Desktop>) -> Entity<Self> {
        if let Some(bar) = bar_for(window, cx) {
            return bar;
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Monitter command"));
        let bar = cx.new(|_| Self {
            input: input.clone(),
            open: false,
            error: None,
            help: false,
            palette: None,
        });
        bar.update(cx, |_, bar_cx| {
            bar_cx
                .subscribe(&input, |this: &mut Self, _, event: &InputEvent, cx| {
                    if matches!(
                        event,
                        InputEvent::PressEnter {
                            secondary: false,
                            shift: false
                        }
                    ) {
                        this.submit(cx);
                    }
                })
                .detach();
        });
        if cx.try_global::<VimCommandRegistry>().is_none() {
            cx.set_global(VimCommandRegistry::default());
        }
        cx.global_mut::<VimCommandRegistry>()
            .bars
            .insert(window.window_handle().window_id(), bar.clone());
        bar
    }

    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = true;
        self.error = None;
        self.help = false;
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.input.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn set_palette(&mut self, palette: crate::design::Palette, cx: &mut Context<Self>) {
        if self.palette != Some(palette) {
            self.palette = Some(palette);
            cx.notify();
        }
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        let source = self.input.read(cx).value().to_string();
        match parse_vim_command(&source) {
            Ok(VimCommand::Help) => {
                self.help = true;
                self.error = None;
            }
            Ok(command) => {
                self.open = false;
                self.error = None;
                cx.emit(VimCommandBarEvent::Run(command));
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }

    pub(crate) fn render_overlay(
        &self,
        palette: crate::design::Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.open {
            return None;
        }
        let mut content = div()
            .id("vim-command-bar")
            .w(px(540.))
            .max_w_full()
            .flex()
            .flex_col()
            .gap(px(6.))
            .p(px(8.))
            .rounded_md()
            .border_1()
            .border_color(rgb(palette.line))
            .bg(crate::design::surface_color(palette.panel))
            .text_color(rgb(palette.ink))
            .on_key_down(cx.listener(|bar, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    bar.open = false;
                    bar.error = None;
                    cx.emit(VimCommandBarEvent::Dismiss);
                    cx.notify();
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .child(":")
                    .child(Input::new(&self.input)),
            );
        if let Some(error) = &self.error {
            content = content.child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(0xc35b5b))
                    .child(error.clone()),
            );
        }
        if self.help {
            content = content.child(
                div()
                    .grid()
                    .grid_cols(2)
                    .gap(px(4.))
                    .text_size(px(10.))
                    .text_color(rgb(palette.muted))
                    .children(
                        [
                            ":tabnew · create a chat draft",
                            ":tabnext / :tabprevious · select a tab",
                            ":split / :vsplit · split the focused pane",
                            ":close · merge this pane into its neighbor",
                            ":only · keep only the focused pane",
                            ":terminal · open a terminal tab",
                            ":wincmd h/j/k/l · focus by direction",
                            ":wincmd H/J/K/L · move the whole pane",
                            ":wincmd r/R · rotate pane contents",
                            ":wincmd x · exchange with next pane",
                            ":wincmd = · equalize pane sizes",
                        ]
                        .into_iter()
                        .map(|line| div().child(line)),
                    ),
            );
        }
        Some(
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom(px(20.))
                .flex()
                .justify_center()
                .child(content)
                .into_any_element(),
        )
    }
}

impl Render for VimCommandBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.palette
            .and_then(|palette| self.render_overlay(palette, cx))
            .unwrap_or_else(|| div().into_any_element())
    }
}

/// Lookup by the actual GPUI window, so detached hosts retain independent input,
/// focus, errors and open state. Creation/event subscriptions remain explicit.
pub fn bar_for(window: &Window, cx: &App) -> Option<Entity<VimCommandBar>> {
    bar_for_id(window.window_handle().window_id(), cx)
}

pub fn bar_for_id(window_id: WindowId, cx: &App) -> Option<Entity<VimCommandBar>> {
    cx.try_global::<VimCommandRegistry>()?
        .bars
        .get(&window_id)
        .cloned()
}

/// Compatibility for callers without a Window. Never selects another host's bar.
pub fn bar(cx: &App) -> Option<Entity<VimCommandBar>> {
    bar_for_id(cx.active_window()?.window_id(), cx)
}

/// Release retained input/entities when their native host closes.
pub fn forget_window(window_id: WindowId, cx: &mut App) {
    if cx.try_global::<VimCommandRegistry>().is_some() {
        cx.global_mut::<VimCommandRegistry>()
            .bars
            .remove(&window_id);
    }
}

pub fn open(cx: &mut Context<crate::Desktop>, window: &mut Window) -> bool {
    let Some(bar) = bar_for(window, cx) else {
        return false;
    };
    bar.update(cx, |bar, cx| bar.open(window, cx));
    true
}

pub fn parse_vim_window_key(key: &str) -> Result<VimCommand, String> {
    if key.len() != 1 {
        return Err("Enter one Ctrl+W command key.".into());
    }
    Ok(match key {
        "h" => VimCommand::FocusPane("left".into()),
        "j" => VimCommand::FocusPane("down".into()),
        "k" => VimCommand::FocusPane("up".into()),
        "l" => VimCommand::FocusPane("right".into()),
        "t" => VimCommand::FocusPane("first".into()),
        "b" => VimCommand::FocusPane("last".into()),
        "w" => VimCommand::FocusPane("next".into()),
        "W" => VimCommand::FocusPane("previous".into()),
        "s" => VimCommand::Split {
            direction: "down".into(),
            size: None,
        },
        "v" => VimCommand::Split {
            direction: "right".into(),
            size: None,
        },
        "c" => VimCommand::ClosePane(None),
        "o" => VimCommand::OnlyPane,
        "r" => VimCommand::Rotate(1),
        "R" => VimCommand::Rotate(-1),
        "x" => VimCommand::Exchange(None),
        "H" => VimCommand::MovePane("left".into()),
        "J" => VimCommand::MovePane("down".into()),
        "K" => VimCommand::MovePane("up".into()),
        "L" => VimCommand::MovePane("right".into()),
        "=" => VimCommand::Equalize,
        _ => return Err(format!("Unsupported Ctrl+W command: {key}.")),
    })
}

pub fn parse_vim_command(source: &str) -> Result<VimCommand, String> {
    if source.len() > 256 {
        return Err("Monitter commands are limited to 256 bytes.".into());
    }
    let source = source.trim().strip_prefix(':').unwrap_or(source.trim());
    if source.is_empty() {
        return Err("Enter a Monitter command, for example :tabnew.".into());
    }
    let mut words = source.split_whitespace();
    let first = words.next().unwrap_or_default();
    let args = words.collect::<Vec<_>>();
    if first.eq_ignore_ascii_case("wincmd") {
        if args.len() != 1 {
            return Err(":wincmd needs exactly one Ctrl+W command key.".into());
        }
        return parse_vim_window_key(args[0]);
    }
    let (prefix, name) = split_prefix(first);
    let arg = match args.as_slice() {
        [] => None,
        [one] => Some(*one),
        _ => return Err(format!(":{name} accepts at most one target or size.")),
    };
    let raw = arg.or(prefix.as_deref());
    let result = match name.to_ascii_lowercase().as_str() {
        "tabnew" | "tabedit" | "tabe" if raw.is_none() => VimCommand::NewChat,
        "tabnext" | "tabn" => VimCommand::SelectTab(parse_target(raw.unwrap_or("+"))?),
        "tabprevious" | "tabp" => VimCommand::SelectTab(parse_target(raw.unwrap_or("-"))?),
        "tabfirst" | "tabrewind" | "tabfir" if raw.is_none() => {
            VimCommand::SelectTab(VimTabTarget::Index(1))
        }
        "tablast" | "tabl" if raw.is_none() => VimCommand::SelectTab(VimTabTarget::Last),
        "tabclose" | "tabc" | "q" | "quit" => {
            VimCommand::CloseTab(raw.map(parse_target).transpose()?)
        }
        "tabonly" | "tabo" if raw.is_none() => VimCommand::OnlyTabs,
        "tabmove" | "tabm" => VimCommand::MoveTab(raw.unwrap_or("$").to_owned()),
        "split" | "sp" => VimCommand::Split {
            direction: "down".into(),
            size: parse_positive(raw, ":split only accepts a positive numeric size.")?,
        },
        "vsplit" | "vs" => VimCommand::Split {
            direction: "right".into(),
            size: parse_positive(raw, ":vsplit only accepts a positive numeric size.")?,
        },
        "terminal" | "term" if raw.is_none() => VimCommand::Terminal,
        "close" | "clo" => VimCommand::ClosePane(
            raw.map(|v| parse_positive_usize(v, ":close only accepts a positive pane number."))
                .transpose()?,
        ),
        "only" | "on" if raw.is_none() => VimCommand::OnlyPane,
        "resize" | "res" => parse_resize("vertical", raw)?,
        "help" if raw.is_none() => VimCommand::Help,
        "vertical" | "vert"
            if args
                .first()
                .is_some_and(|a| a.eq_ignore_ascii_case("resize"))
                && args.len() <= 2 =>
        {
            parse_resize("horizontal", args.get(1).copied())?
        }
        _ => {
            return Err(format!(
                "Unknown or invalid Monitter command: :{name}. Type :help for available commands."
            ));
        }
    };
    Ok(result)
}

fn split_prefix(first: &str) -> (Option<String>, &str) {
    let boundary = first
        .char_indices()
        .find(|(_, ch)| ch.is_ascii_alphabetic())
        .map(|(ix, _)| ix);
    boundary.map_or((None, first), |ix| {
        (Some(first[..ix].to_owned()), &first[ix..])
    })
}
fn parse_target(value: &str) -> Result<VimTabTarget, String> {
    if value == "$" {
        return Ok(VimTabTarget::Last);
    }
    if value == "+" {
        return Ok(VimTabTarget::Relative(1));
    }
    if value == "-" {
        return Ok(VimTabTarget::Relative(-1));
    }
    if let Some(number) = value.strip_prefix('+') {
        return number
            .parse::<isize>()
            .map(|n| VimTabTarget::Relative(n.max(1)))
            .map_err(|_| "Invalid tab target.".into());
    }
    if let Some(number) = value.strip_prefix('-') {
        return number
            .parse::<isize>()
            .map(|n| VimTabTarget::Relative(-n.abs().max(1)))
            .map_err(|_| "Invalid tab target.".into());
    }
    value
        .parse::<usize>()
        .ok()
        .filter(|n| *n > 0)
        .map(VimTabTarget::Index)
        .ok_or_else(|| "Invalid tab target.".into())
}
fn parse_positive(value: Option<&str>, message: &str) -> Result<Option<u32>, String> {
    value
        .map(|raw| {
            raw.parse::<u32>()
                .ok()
                .filter(|n| *n > 0)
                .ok_or_else(|| message.into())
        })
        .transpose()
}
fn parse_positive_usize(value: &str, message: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| message.into())
}
fn parse_resize(axis: &str, raw: Option<&str>) -> Result<VimCommand, String> {
    let Some(raw) = raw else {
        return Ok(VimCommand::Resize {
            axis: axis.into(),
            amount: None,
            maximize: true,
        });
    };
    if let Some(delta) = raw.strip_prefix('+') {
        return Ok(VimCommand::Resize {
            axis: axis.into(),
            amount: Some(
                delta
                    .parse::<isize>()
                    .map_err(|_| "Resize expects a positive size or +N/-N.".to_string())?
                    .max(1),
            ),
            maximize: false,
        });
    }
    if let Some(delta) = raw.strip_prefix('-') {
        return Ok(VimCommand::Resize {
            axis: axis.into(),
            amount: Some(
                -delta
                    .parse::<isize>()
                    .map_err(|_| "Resize expects a positive size or +N/-N.".to_string())?
                    .abs()
                    .max(1),
            ),
            maximize: false,
        });
    }
    let size = raw
        .parse::<isize>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| "Resize expects a positive size or +N/-N.".to_string())?;
    Ok(VimCommand::Resize {
        axis: axis.into(),
        amount: Some(size),
        maximize: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn parses_direct_window_algebra() {
        assert_eq!(
            parse_vim_window_key("s").unwrap(),
            VimCommand::Split {
                direction: "down".into(),
                size: None
            }
        );
        assert_eq!(
            parse_vim_window_key("H").unwrap(),
            VimCommand::MovePane("left".into())
        );
        assert_eq!(parse_vim_window_key("=").unwrap(), VimCommand::Equalize);
        assert_eq!(
            parse_vim_window_key("q").unwrap_err(),
            "Unsupported Ctrl+W command: q."
        );
    }

    #[::core::prelude::v1::test]
    fn parses_bounded_ex_workspace_commands_and_rejects_bad_targets() {
        assert_eq!(
            parse_vim_command(":vsplit 480").unwrap(),
            VimCommand::Split {
                direction: "right".into(),
                size: Some(480)
            }
        );
        assert_eq!(
            parse_vim_command(":wincmd x").unwrap(),
            VimCommand::Exchange(None)
        );
        assert_eq!(
            parse_vim_command(":close 2").unwrap(),
            VimCommand::ClosePane(Some(2))
        );
        assert!(parse_vim_command(":vsplit zero").is_err());
        assert!(parse_vim_command(":only extra args").is_err());
    }
}
