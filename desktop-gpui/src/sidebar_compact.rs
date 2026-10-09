//! Compact, accessible icon rail for the collapsed workspace sidebar.
use gpui_kit::component::tooltip::ManagedTooltipExt as _;
use super::*;
use gpui_kit::{
    assets::__private as icons_assets,
    component::{Icon, Sizable, tooltip::Tooltip, button::*},
    Image,
};
use std::sync::Arc;

pub(super) fn brand_mark() -> Arc<Image> { branding::mark() }

fn icon_button(
    id: impl Into<SharedString>,
    label: &'static str,
    icon: &'static [u8],
    active: bool,
    palette: design::Palette,
    click: impl Fn(&mut Desktop, &mut Window, &mut Context<Desktop>) + 'static,
    cx: &mut Context<Desktop>,
) -> AnyElement {
    Button::new(id.into())
        .compact()
        .icon(Icon::default().data(icon).with_size(design::ui_scaled_px(16.)))
        .tooltip(label)
        .accessibility_label(label)
        .when(active, |button| button.outline())
        .on_click(cx.listener(move |desktop, _, window, cx| click(desktop, window, cx)))
        .into_any_element()
}

pub(super) fn render(
    desktop: &Desktop,
    agents: &[Value],
    tasks: &[Value],
    palette: design::Palette,
    window: &mut Window,
    cx: &mut Context<Desktop>,
) -> AnyElement {
    let width = gpui_kit::px(design::COMPACT_SIDEBAR_WIDTH_POINTS);
    let mut views = div().flex().flex_col().items_center().gap(px(5.));
    for tab in VIEWS {
        let next = tab.view;
        let active = desktop.sidebar_view == tab.view;
        views = views.child(icon_button(
            SharedString::from(format!("compact-sidebar-{}", tab.label.to_ascii_lowercase())),
            tab.label,
            tab.icon,
            active,
            palette,
            move |view, _, cx| { view.sidebar_view = next; cx.notify(); },
            cx,
        ));
    }

    let mut ordered = tasks.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| {
        let left = left.get("updatedAt").and_then(Value::as_i64).unwrap_or_default();
        let right = right.get("updatedAt").and_then(Value::as_i64).unwrap_or_default();
        right.cmp(&left)
    });
    let chat_scroll = window.use_keyed_state("compact-chat-overflow", cx,
        |_, _| ScrollHandle::new()).read(cx).clone();
    let mut chat_icons = div().id("compact-chat-icons").flex_1().min_h_0().overflow_y_scroll().track_scroll(&chat_scroll).flex().flex_col().items_center().gap(px(4.));
    for task in ordered {
        let id = field(task, "id");
        let title = field(task, "title");
        let agent = agents.iter().find(|agent| field(agent, "id") == field(task, "agentId"));
        let provider = field(task, "provider");
        let model = field(task, "model");
        let active = desktop.selected.as_ref() == Some(&id);
        let hover_key = format!("compact-chat-{id}");
        let (hover_state, background_opacity) =
            sidebar_hover::state(&hover_key, active, window, cx);
        let task_id = id.clone();
        let icon = crate::branding::provider_for_ink(&provider, &model, palette.ink)
            .map(|image| img(image).w(px(17.)).h(px(17.)).into_any_element())
            .unwrap_or_else(|| Icon::default().data(icons_assets::MessageSquare.1)
                .with_size(px(16.)).into_any_element());
        let status = field(task, "status");
        let avatar_color = agent.map(|agent| design::parse_color(&field(agent, "color"), palette.accent)).unwrap_or(palette.muted);
        let tooltip_label = format!("{title} · {provider} · {status}");
        // Managed tooltip owns an on_hover slot. Keep it on a separate
        // full-size wrapper so the retained background animation has its own.
        chat_icons = chat_icons.child(
            div()
                .id(SharedString::from(format!("{hover_key}-tooltip")))
                .w(width - px(8.))
                .h(px(34.))
                .managed_tooltip(move |window, cx| Tooltip::new(tooltip_label.clone()).build(window, cx))
                .child(div()
                .id(SharedString::from(hover_key))
                .w(width - px(8.))
                .h(px(34.))
                .rounded_md()
                .cursor_pointer()
                .flex()
                .items_center()
                .justify_center()
                .relative()
                .bg(crate::design::surface_color(palette.soft).opacity(background_opacity))
                .overflow_hidden()
                .children(crate::activity_effects::for_task(&task_id, "minibar", crate::activity_sparkles::SparkleLayout::Compact, false, cx))
                .child(icon)
                .child(div().absolute().right_1().bottom_1().w(px(6.)).h(px(6.)).rounded_full()
                    .bg(rgb(if status == "running" { design::status_color(&status, palette) } else { avatar_color })))
                .aria_label(format!("Open chat {}", field(task, "title")))
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.switch_surface(Sidebar::Chats, window, cx);
                    view.selected = Some(task_id.clone());
                    cx.notify();
                }))
                .on_hover(cx.listener(move |_, hovered, _, cx| {
                    sidebar_hover::set(&hover_state, *hovered, cx);
                }))),
        );
    }
    if tasks.is_empty() {
        chat_icons = chat_icons.child(note_compact("No chats", palette));
    }

    let footer_actions = [
        (Sidebar::Chats, "New chat", icons_assets::MessageSquarePlus.1),
        (Sidebar::Terminal, "New terminal", icons_assets::Terminal.1),
        (Sidebar::Subagents, "Companion mode", icons_assets::Bot.1),
        (Sidebar::Settings, "Settings", icons_assets::Settings.1),
    ];
    let mut footer = div().flex().flex_col().items_center().gap(px(4.));
    for (surface, label, icon) in footer_actions {
        footer = footer.child(icon_button(
            SharedString::from(format!("compact-footer-{}", label.to_ascii_lowercase().replace(' ', "-"))),
            label,
            icon,
            desktop.sidebar == surface,
            palette,
            move |view, window, cx| {
                if surface == Sidebar::Chats { view.open_chat_draft(None, window, cx); }
                else { view.switch_surface(surface, window, cx); }
                cx.notify();
            },
            cx,
        ));
    }

    let _ = window;
    div()
        .id("sidebar-compact-rail")
        .w(width)
        .h_full()
        .flex_shrink_0()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(7.))
        .pb(px(7.))
        // Native traffic lights occupy the top 44 physical pixels.
        .pt(gpui_kit::px(44.))
        .border_color(rgb(palette.line))
        .bg(crate::surface_blend::background(crate::surface_blend::Area::Sidebar, palette.sidebar))
        .child(icon_button("compact-sidebar-expand", "Expand sidebar", icons_assets::ChevronRight.1,
            false, palette, |view, window, cx| view.toggle_sidebar(window, cx), cx))
        .child(div().id("compact-brand-mark").w(px(32.)).h(px(32.)).flex().items_center().justify_center()
            .managed_tooltip(|window, cx| Tooltip::new("Monitter").build(window, cx)).aria_label("Monitter")
            .child(img(brand_mark()).w(px(23.)).h(px(23.))))
        .child(views)
        .child(div().h(px(1.)).w(px(28.)).bg(rgb(palette.line)))
        .child(ScrollEdgeFade::new(chat_icons, chat_scroll, Axis::Vertical, gpui_kit::px(20.)))
        .child(div().h(px(1.)).w(px(28.)).bg(rgb(palette.line)))
        .child(footer)
        .into_any_element()
}

fn note_compact(label: &'static str, palette: design::Palette) -> AnyElement {
    div().text_size(px(8.)).text_color(rgb(palette.muted)).child(label).into_any_element()
}
