// A narrow WKNavigationDelegate adapter for Wry 0.55.1. Wry owns the original
// delegate; the WKWebView owns these adapters until monitter_browser_auth_detach.
// Keep the Wry version pin and review this forwarding contract on upgrades.
#import <AppKit/AppKit.h>
#import <WebKit/WebKit.h>
#import <objc/runtime.h>

typedef void (*MonitterBrowserFocusCallback)(const char *tabId);

@interface MonitterBrowserAuthDelegate : NSObject <WKNavigationDelegate>
@property (nonatomic, weak) id<WKNavigationDelegate> original;
@property (nonatomic, strong) NSMutableSet<NSAlert *> *pendingAlerts;
@property (nonatomic, assign) BOOL detached;
- (instancetype)initWithOriginal:(id<WKNavigationDelegate>)original;
- (void)cancelPending;
@end

@implementation MonitterBrowserAuthDelegate

- (instancetype)initWithOriginal:(id<WKNavigationDelegate>)original {
    self = [super init];
    if (self) {
        _original = original;
        _pendingAlerts = [NSMutableSet set];
    }
    return self;
}

- (BOOL)respondsToSelector:(SEL)selector {
    return [super respondsToSelector:selector] || [self.original respondsToSelector:selector];
}

- (id)forwardingTargetForSelector:(SEL)selector {
    id<WKNavigationDelegate> original = self.original;
    if ([original respondsToSelector:selector]) {
        return original;
    }
    return [super forwardingTargetForSelector:selector];
}

- (void)cancelPending {
    self.detached = YES;
    for (NSAlert *alert in [self.pendingAlerts copy]) {
        NSWindow *sheet = alert.window;
        NSWindow *parent = sheet.sheetParent;
        if (parent) {
            [parent endSheet:sheet returnCode:NSModalResponseCancel];
        }
    }
}

- (void)webView:(WKWebView *)webView
    didReceiveAuthenticationChallenge:(NSURLAuthenticationChallenge *)challenge
                 completionHandler:(void (^)(NSURLSessionAuthChallengeDisposition,
                                              NSURLCredential * _Nullable))completionHandler {
    NSURLProtectionSpace *space = challenge.protectionSpace;
    // TLS trust, client certificates, proxy authentication and other mechanisms
    // remain with WebKit's normal authentication handling.
    if (![space.authenticationMethod isEqualToString:NSURLAuthenticationMethodHTTPBasic]) {
        completionHandler(NSURLSessionAuthChallengePerformDefaultHandling, nil);
        return;
    }
    // A failed login must not create an unbounded stream of sheets.
    if (self.detached || challenge.previousFailureCount >= 3 || self.pendingAlerts.count != 0) {
        completionHandler(NSURLSessionAuthChallengeCancelAuthenticationChallenge, nil);
        return;
    }

    NSWindow *window = webView.window;
    if (!window) {
        completionHandler(NSURLSessionAuthChallengeCancelAuthenticationChallenge, nil);
        return;
    }

    NSAlert *alert = [[NSAlert alloc] init];
    alert.messageText = @"Sign in to website";
    NSString *host = space.host ?: @"Website";
    NSString *realm = space.realm;
    alert.informativeText = realm.length ? [NSString stringWithFormat:@"%@ — %@", host, realm] : host;
    [alert addButtonWithTitle:@"Sign In"];
    [alert addButtonWithTitle:@"Cancel"];

    NSView *fields = [[NSView alloc] initWithFrame:NSMakeRect(0, 0, 320, 72)];
    NSTextField *username = [[NSTextField alloc] initWithFrame:NSMakeRect(0, 39, 320, 27)];
    username.placeholderString = @"Username";
    username.stringValue = challenge.proposedCredential.user ?: @"";
    NSSecureTextField *password = [[NSSecureTextField alloc] initWithFrame:NSMakeRect(0, 4, 320, 27)];
    password.placeholderString = @"Password";
    [fields addSubview:username];
    [fields addSubview:password];
    alert.accessoryView = fields;

    [self.pendingAlerts addObject:alert];
    __weak MonitterBrowserAuthDelegate *weakSelf = self;
    __weak NSAlert *weakAlert = alert;
    [alert beginSheetModalForWindow:window completionHandler:^(NSModalResponse response) {
        MonitterBrowserAuthDelegate *delegate = weakSelf;
        NSAlert *finishedAlert = weakAlert;
        if (finishedAlert) {
            [delegate.pendingAlerts removeObject:finishedAlert];
        }
        if (!delegate || delegate.detached || response != NSAlertFirstButtonReturn) {
            completionHandler(NSURLSessionAuthChallengeCancelAuthenticationChallenge, nil);
            return;
        }
        // Session-only storage allows ordinary reloads without persisting a
        // password in Monitter's profile, logs, events or application state.
        NSURLCredential *credential = [NSURLCredential credentialWithUser:username.stringValue
                                                                password:password.stringValue
                                                             persistence:NSURLCredentialPersistenceForSession];
        completionHandler(NSURLSessionAuthChallengeUseCredential, credential);
    }];
}

@end

static const char monitterAuthAssociationKey;
static const char monitterFocusAssociationKey;
static id monitterFocusMonitor;
static MonitterBrowserFocusCallback monitterFocusCallback;
static NSUInteger monitterFocusRegistrationCount;

static WKWebView *monitterBrowserWebView(void *rawWebView) {
    if (![NSThread isMainThread] || !rawWebView) return nil;
    id object = (__bridge id)rawWebView;
    return [object isKindOfClass:[WKWebView class]] ? (WKWebView *)object : nil;
}

// Both functions must run on AppKit's main thread (Tauri with_webview callback).
// One process-local monitor serves every browser child. It walks only the
// clicked view's ancestors and reads a tab ID associated with a WKWebView,
// rather than installing an O(tab-count) monitor set.
bool monitter_browser_auth_attach(void *rawWebView, const char *tabId, MonitterBrowserFocusCallback focusCallback) {
    WKWebView *webView = monitterBrowserWebView(rawWebView);
    if (!webView || !tabId || !focusCallback) return false;
    if (objc_getAssociatedObject(webView, &monitterAuthAssociationKey)) return true;

    id<WKNavigationDelegate> original = webView.navigationDelegate;
    if (!original || [original isKindOfClass:[MonitterBrowserAuthDelegate class]]) return false;
    MonitterBrowserAuthDelegate *adapter = [[MonitterBrowserAuthDelegate alloc] initWithOriginal:original];
    objc_setAssociatedObject(webView, &monitterAuthAssociationKey, adapter, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    webView.navigationDelegate = adapter;

    NSString *focusTabId = [NSString stringWithUTF8String:tabId];
    if (!focusTabId) {
        webView.navigationDelegate = original;
        objc_setAssociatedObject(webView, &monitterAuthAssociationKey, nil, OBJC_ASSOCIATION_ASSIGN);
        return false;
    }
    if (!monitterFocusMonitor) {
        monitterFocusCallback = focusCallback;
        monitterFocusMonitor = [NSEvent addLocalMonitorForEventsMatchingMask:(NSEventMaskLeftMouseDown | NSEventMaskRightMouseDown | NSEventMaskOtherMouseDown)
                                                                      handler:^NSEvent *(NSEvent *event) {
            NSWindow *window = event.window;
            NSView *contentView = window.contentView;
            if (!contentView || !monitterFocusCallback) return event;
            // locationInWindow is in the window base coordinate system; convert
            // it to content-view coordinates before hit testing.
            NSPoint point = [contentView convertPoint:event.locationInWindow fromView:nil];
            NSView *hitView = [contentView hitTest:point];
            for (NSView *view = hitView; view; view = view.superview) {
                NSString *clickedTabId = objc_getAssociatedObject(view, &monitterFocusAssociationKey);
                if (clickedTabId) {
                    monitterFocusCallback(clickedTabId.UTF8String);
                    break;
                }
            }
            return event;
        }];
        if (!monitterFocusMonitor) {
            monitterFocusCallback = NULL;
            webView.navigationDelegate = original;
            objc_setAssociatedObject(webView, &monitterAuthAssociationKey, nil, OBJC_ASSOCIATION_ASSIGN);
            return false;
        }
    }
    // The view owns only its immutable tab ID. The single monitor is retained
    // globally and removed when the final browser child detaches.
    objc_setAssociatedObject(webView, &monitterFocusAssociationKey, focusTabId, OBJC_ASSOCIATION_COPY_NONATOMIC);
    monitterFocusRegistrationCount += 1;
    return true;
}

void monitter_browser_auth_detach(void *rawWebView) {
    WKWebView *webView = monitterBrowserWebView(rawWebView);
    if (!webView) return;
    MonitterBrowserAuthDelegate *adapter = objc_getAssociatedObject(webView, &monitterAuthAssociationKey);
    if (objc_getAssociatedObject(webView, &monitterFocusAssociationKey)) {
        objc_setAssociatedObject(webView, &monitterFocusAssociationKey, nil, OBJC_ASSOCIATION_ASSIGN);
        if (monitterFocusRegistrationCount > 0) monitterFocusRegistrationCount -= 1;
        if (monitterFocusRegistrationCount == 0 && monitterFocusMonitor) {
            [NSEvent removeMonitor:monitterFocusMonitor];
            monitterFocusMonitor = nil;
            monitterFocusCallback = NULL;
        }
    }
    if (!adapter) return;
    [adapter cancelPending];
    if (webView.navigationDelegate == adapter) {
        webView.navigationDelegate = adapter.original;
    }
    objc_setAssociatedObject(webView, &monitterAuthAssociationKey, nil, OBJC_ASSOCIATION_ASSIGN);
}

// History calls are kept in the same tiny native bridge so Rust never sends
// Objective-C messages to an unvalidated pointer. Return false on invalid
// view/thread; an empty history is a successful query with both outputs false.
bool monitter_browser_history_state(void *rawWebView, bool *canGoBack, bool *canGoForward) {
    WKWebView *webView = monitterBrowserWebView(rawWebView);
    if (!webView || !canGoBack || !canGoForward) return false;
    *canGoBack = webView.canGoBack;
    *canGoForward = webView.canGoForward;
    return true;
}

bool monitter_browser_go_back(void *rawWebView) {
    WKWebView *webView = monitterBrowserWebView(rawWebView);
    return webView && webView.canGoBack && [webView goBack] != nil;
}

bool monitter_browser_go_forward(void *rawWebView) {
    WKWebView *webView = monitterBrowserWebView(rawWebView);
    return webView && webView.canGoForward && [webView goForward] != nil;
}
