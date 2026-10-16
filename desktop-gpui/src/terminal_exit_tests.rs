//! Exercise real native dock removal through the terminal exit event boundary.
//! No service/profile/backend is created by this fixture.
use super::{TerminalEvent, remove_exited_terminal_pane};
use crate::docking::{DockWorkspace, PaneFactory, PaneSpec, PaneLifecycle};
use gpui_kit::{AppContext as _, Context, EventEmitter, IntoElement, Render, Styled as _, TestAppContext, Window, div};
use gpui_kit::component::dock::DockPlacement;
use std::{cell::RefCell, rc::Rc};
struct Content;
impl Render for Content {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {div().size_full()}
}
impl EventEmitter<TerminalEvent> for Content {}
#[gpui_kit::test]
fn terminal_exit_event_removes_only_its_hidden_native_pane(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let mut dock=None;
    let mut source=None;
    let events=Rc::new(RefCell::new(Vec::new()));
    let (_,visual)=cx.add_window_view(|window,cx| {
        let factory:PaneFactory=Rc::new(|_,_,_|None);
        let workspace=cx.new(|cx|DockWorkspace::new("terminal-exit-fixture",factory,window,cx));
        let terminal=cx.new(|_|Content);
        let target=workspace.clone();
        cx.subscribe_in(&terminal,window,move |_,_,event:&TerminalEvent,window,cx| {
            let session_id=match event {TerminalEvent::Exited {session_id}|TerminalEvent::SessionClosed {session_id}=>session_id};
            remove_exited_terminal_pane(&target,session_id,window,cx);
        }).detach();
        workspace.update(cx,|dock,cx| {
            for (id,content) in [("terminal:11111111-1111-4111-8111-111111111111",terminal.clone()),("terminal:22222222-2222-4222-8222-222222222222",cx.new(|_|Content))] {
                let capture=events.clone();
                let identity=id.to_owned();
                dock.add(PaneSpec {id:id.into(),title:id.into(),content:content.into(),focus:cx.focus_handle(),closable:true,lifecycle:Some(Rc::new(move |event,_,_|capture.borrow_mut().push((identity.clone(),event))))},DockPlacement::Center,None,window,cx).unwrap();
            }
            dock.set_shown(false,window,cx);
        });
        dock=Some(workspace.clone());source=Some(terminal);
        gpui_kit::component::Root::new(workspace,window,cx)
    });
    let dock=dock.unwrap();let source=source.unwrap();
    visual.update(|_,cx|source.update(cx,|_,cx|cx.emit(TerminalEvent::Exited {session_id:"11111111-1111-4111-8111-111111111111".into()})));
    visual.run_until_parked();
    visual.update(|_,cx| {
        assert!(!dock.read(cx).contains("terminal:11111111-1111-4111-8111-111111111111"));
        assert!(dock.read(cx).contains("terminal:22222222-2222-4222-8222-222222222222"));
        let events=events.borrow();
        assert!(events.iter().any(|(id,event)|id=="terminal:11111111-1111-4111-8111-111111111111" && matches!(event,PaneLifecycle::Removed)));
        assert!(!events.iter().any(|(id,event)|id=="terminal:22222222-2222-4222-8222-222222222222" && matches!(event,PaneLifecycle::Removed)));
    });
    // A positive manager Close acknowledgement may purge its backend record;
    // it uses the same exact-identity removal without needing a missing-list guess.
    visual.update(|_,cx|source.update(cx,|_,cx|cx.emit(TerminalEvent::SessionClosed {session_id:"22222222-2222-4222-8222-222222222222".into()})));
    visual.run_until_parked();
    visual.update(|_,cx|assert!(!dock.read(cx).contains("terminal:22222222-2222-4222-8222-222222222222")));

}
