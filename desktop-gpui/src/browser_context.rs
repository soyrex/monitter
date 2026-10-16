//! Scoped native context-click interception. No script message handler or page IPC.

pub(super) const NEW_PANE: &str = "Open in a new pane";
pub(super) const SYSTEM: &str = "Open in system browser";
pub(super) const COPY: &str = "Copy link";
pub(super) fn items(link: bool) -> &'static [&'static str] {
    if link { &[NEW_PANE, SYSTEM, COPY] } else { &[SYSTEM] }
}
pub(super) fn placement(point: (f32,f32), pane: (f32,f32), desired_width: f32, desired_height: f32) -> (f32,f32,f32,f32) {
    let width = desired_width.min((pane.0-8.).max(0.));
    let height = desired_height.min((pane.1-8.).max(0.));
    let left = (point.0*pane.0).clamp(4.,(pane.0-width-4.).max(4.));
    let top = (point.1*pane.1).clamp(4.,(pane.1-height-4.).max(4.));
    (left,top,width,height)
}

pub(super) fn current(eligible: bool, generation: u64, request: u64, location: &str, source: &str) -> bool {
    eligible && generation == request && location == source
}

pub(super) fn target(value: &str) -> Option<String> {
    if value.len() > 16 * 1024 || value.chars().any(char::is_control) { return None; }
    let url = super::validated_url(value).ok()?;
    ((matches!(url.scheme(), "http" | "https") && url.host_str().is_some()) || url.scheme() == "file").then(|| url.to_string())
}

/// A remote page cannot turn an untrusted hit-test result into a local-file action.
/// Local HTML reports retain the browser's existing local HTML URL policy.
pub(super) fn target_for_page(value: &str, source: &str) -> Option<String> {
    let target = target(value)?;
    if target.starts_with("file:") && super::validated_url(source).ok()?.scheme() != "file" {
        return None;
    }
    Some(target)
}

/// Accept only finite points inside this native view's own bounds. AppKit's
/// contentView hitTest can return GPUI's root rather than its WKWebView child.
fn normalized_point(point: (f64,f64), origin: (f64,f64), size: (f64,f64), flipped: bool) -> Option<(f64,f64)> {
    if ![point.0,point.1,origin.0,origin.1,size.0,size.1].into_iter().all(f64::is_finite)
        || size.0 <= 0. || size.1 <= 0. { return None; }
    let x=(point.0-origin.0)/size.0;
    let y=(point.1-origin.1)/size.1;
    if !(0.0..1.0).contains(&x) || !(0.0..1.0).contains(&y) { return None; }
    Some((x,if flipped { y } else { 1.-y }))
}

pub(super) fn hit_test(x: f64, y: f64) -> String {
    // Fractions account for native bounds, zoom, and CSS viewport scaling. Closed
    // shadow roots and cross-origin frames deliberately produce no link result.
    format!(r#"(() => {{
      function hit(d,x,y) {{
        let e=d.elementFromPoint(x,y); if(!e)return {{kind:'page'}};
        for(let n=0;n<32;n++) {{
          if(e.shadowRoot) {{let s=e.shadowRoot.elementFromPoint(x,y);if(s&&s!==e){{e=s;continue;}}}}
          if(e.localName==='iframe'||e.localName==='frame') {{
            try {{let r=e.getBoundingClientRect(),w=e.contentWindow,doc=e.contentDocument;
              if(!doc)return {{kind:'blocked'}};
              return hit(doc,(x-r.left-e.clientLeft)*w.innerWidth/e.clientWidth,(y-r.top-e.clientTop)*w.innerHeight/e.clientHeight);
            }} catch(_) {{return {{kind:'blocked'}};}}
          }}
          let a=e.closest?.('a[href],area[href]');
          if(a) return {{kind:'link',url:new URL(a.getAttribute('href'),a.baseURI).href}};
          let host=e.getRootNode?.().host;if(!host)break;e=host;
        }}
        return {{kind:'page'}};
      }}
      try {{return hit(document,innerWidth*{x},innerHeight*{y});}}catch(_){{return {{kind:'blocked'}};}}
    }})()"#)
}

#[cfg(target_os="macos")]
mod native {
    use objc2::{class,msg_send,rc::Retained,runtime::AnyObject};
    use objc2_foundation::{NSPoint,NSRect};
    use wry::WebViewExtMacOS;
    pub(super) struct Monitor(Retained<AnyObject>);
    impl Drop for Monitor { fn drop(&mut self) { unsafe { let _: ()=msg_send![class!(NSEvent),removeMonitor:&*self.0]; } } }
    pub(super) fn install(view:&wry::WebView)->(Option<Monitor>,futures::channel::mpsc::UnboundedReceiver<(f64,f64)>) {
        let (sender,receiver)=futures::channel::mpsc::unbounded();
        let native=view.webview();
        let callback=block2::RcBlock::new(move |event:*mut AnyObject|->*mut AnyObject { unsafe {
            if event.is_null(){return event;}
            let window:*mut AnyObject=msg_send![&*native,window];
            let event_window:*mut AnyObject=msg_send![event,window];
            let hidden:bool=msg_send![&*native,isHiddenOrHasHiddenAncestor];
            if window.is_null()||window!=event_window||hidden{return event;}
            let kind:usize=msg_send![event,type];let flags:usize=msg_send![event,modifierFlags];
            if kind!=3 && !(kind==1 && flags&(1<<18)!=0){return event;}
            let p:NSPoint=msg_send![event,locationInWindow];
            let p:NSPoint=msg_send![&*native,convertPoint:p,fromView:std::ptr::null::<AnyObject>()];
            let bounds:NSRect=msg_send![&*native,bounds];
            let flipped:bool=msg_send![&*native,isFlipped];
            let Some(point)=super::normalized_point((p.x,p.y),(bounds.origin.x,bounds.origin.y),
                (bounds.size.width,bounds.size.height),flipped) else { return event; };
            if sender.unbounded_send(point).is_ok(){std::ptr::null_mut()}else{event}
        }});
        let monitor=unsafe {let raw:*mut AnyObject=msg_send![class!(NSEvent),addLocalMonitorForEventsMatchingMask:((1usize<<3)|(1usize<<1)),handler:&*callback];Retained::retain(raw).map(Monitor)};
        (monitor,receiver)
    }
}
pub(super) struct ContextClick { #[cfg(target_os="macos")] _monitor:Option<native::Monitor> }
impl ContextClick {
    pub(super) fn install(view:&wry::WebView)->(Self,futures::channel::mpsc::UnboundedReceiver<(f64,f64)>) {
        #[cfg(target_os="macos")] {let (monitor,receiver)=native::install(view);(Self{_monitor:monitor},receiver)}
        #[cfg(not(target_os="macos"))] {let _=view;let (_,receiver)=futures::channel::mpsc::unbounded();(Self{},receiver)}
    }
}
#[cfg(test)] mod tests {
    #[test] fn native_clicks_must_be_inside_finite_view_bounds() {
        assert_eq!(super::normalized_point((60.,45.),(10.,20.),(100.,100.),true),Some((0.5,0.25)));
        assert_eq!(super::normalized_point((60.,45.),(10.,20.),(100.,100.),false),Some((0.5,0.75)));
        for point in [(9.,45.),(110.,45.),(60.,19.),(60.,120.),(f64::NAN,45.),(60.,f64::INFINITY)] {
            assert!(super::normalized_point(point,(10.,20.),(100.,100.),true).is_none());
        }
        for size in [(0.,100.),(100.,-1.),(f64::INFINITY,100.)] {
            assert!(super::normalized_point((10.,20.),(10.,20.),size,true).is_none());
        }
        assert!(super::normalized_point((10.,20.),(f64::NAN,20.),(100.,100.),true).is_none());
    }
    #[test] fn placement_keeps_full_menu_inside_narrow_panes() {
        for width in [280.,600.] { for point in [(0.02,0.02),(0.49,0.49),(0.98,0.98)] {
            let (x,y,w,h)=super::placement(point,(width,320.),220.,108.);
            assert_eq!(w,220.);assert_eq!(h,108.);
            assert!(x>=4. && y>=4. && x+w<=width-4. && y+h<=316.);
        }}
    }
    #[test] fn exact_items() {
        assert_eq!(super::items(true), &["Open in a new pane", "Open in system browser", "Copy link"]);
        assert_eq!(super::items(false), &["Open in system browser"]);
    }
    #[test] fn stale_requests_never_reopen_a_menu() {
        assert!(super::current(true,3,3,"a","a"));
        assert!(!super::current(false,3,3,"a","a"));
        assert!(!super::current(true,4,3,"a","a"));
        assert!(!super::current(true,3,3,"b","a"));
    }
    #[test] fn remote_pages_cannot_supply_local_file_targets() {
        let file="file:///tmp/report.html";
        assert!(super::target_for_page(file,"https://example.com/").is_none());
        assert!(super::target_for_page(file,"http://localhost/").is_none());
        assert!(super::target_for_page(file,"about:blank").is_none());
        assert!(super::target_for_page(file,"file:///tmp/index.html").is_some());
        assert!(super::target_for_page("https://example.com/","file:///tmp/index.html").is_some());
        assert!(super::target_for_page("https://example.com/","https://other.example/").is_some());
        assert!(super::target_for_page("file:///etc/passwd","file:///tmp/index.html").is_none());
        assert!(super::target_for_page("javascript:alert(1)","file:///tmp/index.html").is_none());
    }
    #[test] fn only_web_targets() {
        for bad in ["javascript:alert(1)","data:text/plain,a","https://example.com/\n", "mailto:a@b.c"] {assert!(super::target(bad).is_none());}
        assert!(super::target(&format!("https://example.com/{}","a".repeat(16384))).is_none());
        assert!(super::target("file:///tmp/report.html").is_some());
        assert_eq!(super::target("https://example.com"),Some("https://example.com/".into()));
    }
}
