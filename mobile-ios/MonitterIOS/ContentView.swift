import SwiftUI
import WebKit
import Network
import Speech
import AVFoundation
import CryptoKit
import Security

private enum ConnectionMode: Equatable {
    case chooser
    case paired
    case lan(URL)
}

struct ContentView: View {
    @AppStorage("ownerLANBaseURL") private var savedLANAddress = ""
    @AppStorage("lastConnectionMode") private var savedMode = ""
    @AppStorage("persistentPairingFlowVersion") private var pairingFlowVersion = 0
    @Environment(\.scenePhase) private var scenePhase
    @StateObject private var connectionRecovery = WebConnectionRecovery()
    @State private var mode: ConnectionMode = .paired
    @State private var address = ""
    @State private var scannerPresented = false
    @State private var webView: WKWebView?
    @State private var webURL: URL?
    @State private var pairedURL: URL?
    @State private var server: BundleWebServer?
    @State private var startError: String?
    @State private var pairedError: String?
    @State private var pageError: String?
    @State private var editingAddress = false

    var body: some View {
        Group {
            switch mode {
            case .chooser:
                connectionChooser
            case .paired, .lan:
                GeometryReader { geometry in
                    ZStack(alignment: .topTrailing) {
                        WebContentView(webView: $webView, url: webURL, error: startError, webViewReady: { view in
                            connectionRecovery.setWebView(view)
                        }, requestScan: {
                            scannerPresented = true
                        }, requestConnectionChooser: {
                            editingAddress = true
                        }, navigationError: { pageError = $0 })
                        .frame(
                            width: geometry.size.width,
                            height: geometry.size.height + geometry.safeAreaInsets.top + geometry.safeAreaInsets.bottom
                        )
                        .offset(y: -geometry.safeAreaInsets.top)

                        if let currentPageError = pageError {
                            VStack(spacing: 12) {
                                Text("Workspace unavailable").font(.headline)
                                Text(currentPageError).font(.callout).multilineTextAlignment(.center)
                                Button("Retry") { pageError = nil; if let webURL { webView?.load(URLRequest(url: webURL)) } }
                                Button("Change connection") { editingAddress = true }
                            }
                            .padding(20)
                            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 18))
                            .frame(maxWidth: 340)
                            .frame(maxWidth: .infinity, maxHeight: .infinity)
                        } else if case .paired = mode, let pairedError {
                            VStack(spacing: 12) {
                                Text("Relay workspace unavailable").font(.headline)
                                Text(pairedError).font(.callout).multilineTextAlignment(.center)
                                Button("Retry relay workspace") { retryPairedMode() }
                                Button("Change connection") { editingAddress = true }
                            }
                            .padding(20)
                            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 18))
                            .frame(maxWidth: 340)
                            .frame(maxWidth: .infinity, maxHeight: .infinity)
                        }

                        // The live LAN page may be older than this iPhone build.
                        // Keep its escape hatch until the Settings action is
                        // published there; paired mode uses the bundled page.
                        if case .lan = mode {
                            Button { editingAddress = true } label: {
                                Image(systemName: "rectangle.connected.to.line.below")
                                    .font(.system(size: 16, weight: .semibold))
                                    .padding(11)
                                    .background(.regularMaterial, in: Circle())
                            }
                            .accessibilityLabel("Change connection")
                            .padding(.top, 8)
                            .padding(.trailing, 12)
                        }

                    }
                    .frame(width: geometry.size.width, height: geometry.size.height, alignment: .top)
                }
            }
        }
        .task {
            // Migrate old manual-network launches to the bundled QR flow once.
            // Retain the old address in Settings, and respect later explicit choices.
            if pairingFlowVersion < 1 {
                pairingFlowVersion = 1
                savedMode = "paired"
                mode = .paired
            } else if savedMode == "lan", !savedLANAddress.isEmpty, let url = LANAddress.parse(savedLANAddress) {
                mode = .lan(url)
                webURL = url
            } else {
                mode = .paired
            }
            startServerIfNeeded()
        }
        .task(id: webURL) {
            connectionRecovery.setTrustedURL(webURL)
        }
        .onChange(of: scenePhase) { _, phase in
            if phase == .active { connectionRecovery.resumeAfterForeground() }
        }
        .sheet(isPresented: $editingAddress) { connectionChooser }
        .fullScreenCover(isPresented: $scannerPresented) {
            QRScannerView(onScan: { invitation in
                scannerPresented = false
                DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(300)) {
                    connectionRecovery.deliverPairingInvitation(invitation)
                }
            }, onCancel: {
                scannerPresented = false
            })
        }
    }

    private var connectionChooser: some View {
        NavigationStack {
            Form {
                Section("Paired owner workspace") {
                    Text("Your saved phone pairing reconnects through the Monitter relay wherever this phone has internet access. No network or IP address selection is needed.")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                    Button("Use paired workspace") { usePairedMode() }
                }
                Section("Advanced: local network workspace") {
                    TextField("http://192.168.1.20:18436", text: $address)
                        .keyboardType(.URL)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    Text("Enter the Monitter desktop address on your local network. HTTP is accepted for private IPv4 and .local addresses; HTTPS requires port 18436.")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                    Button("Use saved local workspace") { connectToLAN() }
                        .disabled(LANAddress.parse(address) == nil)
                    Text("This only uses the explicitly saved local address. Away from that LAN, return to the paired workspace; it keeps the existing pairing and trust decision.")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }
                if let startError {
                    Section("Connection status") {
                        Text(startError).foregroundStyle(.red)
                        Button("Retry relay workspace") { startServerIfNeeded() }
                    }
                }
            }
            .navigationTitle("Connect Monitter")
            .toolbar {
                if editingAddress {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("Close") { editingAddress = false }
                    }
                }
            }
        }
        .onAppear { address = savedLANAddress }
    }

    private func connectToLAN() {
        guard let url = LANAddress.parse(address) else { return }
        savedLANAddress = url.absoluteString
        savedMode = "lan"
        mode = .lan(url)
        webURL = url
        startError = nil
        pairedError = nil
        pageError = nil
        editingAddress = false
    }

    private func usePairedMode() {
        savedMode = "paired"
        mode = .paired
        editingAddress = false
        startError = nil
        pairedError = nil
        pageError = nil
        webURL = pairedURL
        if pairedURL == nil { startServerIfNeeded() }
    }

    private func retryPairedMode() {
        guard case .paired = mode else { return }
        server?.cancel()
        server = nil
        pairedURL = nil
        webURL = nil
        startError = nil
        pairedError = nil
        pageError = nil
        startServerIfNeeded()
    }

    private func startServerIfNeeded() {
        guard server == nil else { return }
        guard let webRoot = Bundle.main.url(forResource: "Web", withExtension: nil) else {
            startError = "Monitter mobile build missing. Run mobile-ios/build.sh before building the app."
            if case .paired = mode { pairedError = startError }
            return
        }
        do {
            let newServer = try BundleWebServer(root: webRoot)
            server = newServer
            newServer.start(onReady: { url in
                guard self.server === newServer else { return }
                pairedURL = url
                if case .paired = mode {
                    startError = nil
                    pairedError = nil
                    webURL = url
                }
            }, onFailure: { message in
                guard self.server === newServer else { return }
                newServer.cancel()
                self.server = nil
                self.pairedURL = nil
                startError = "Paired mode is unavailable: \(message)"
                if case .paired = mode {
                    webURL = nil
                    pairedError = startError
                }
            })
        } catch {
            startError = "Unable to start paired mode: \(error.localizedDescription)"
            if case .paired = mode { pairedError = startError }
        }
    }
}

private enum LANAddress {
    static func parse(_ input: String) -> URL? {
        let value = input.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let parts = URLComponents(string: value),
              let scheme = parts.scheme?.lowercased(),
              let host = parts.host?.lowercased(),
              parts.user == nil, parts.password == nil,
              parts.query == nil, parts.fragment == nil,
              parts.path.isEmpty || parts.path == "/",
              parts.port == 18436 else { return nil }

        if scheme == "http" {
            guard isPrivateIPv4(host) || (host.hasSuffix(".local") && isHostnameOrIPv4(host)) else { return nil }
        } else if scheme == "https" {
            guard isHostnameOrIPv4(host) else { return nil }
        } else { return nil }

        var normalized = parts
        normalized.path = ""
        return normalized.url
    }

    private static func isPrivateIPv4(_ host: String) -> Bool {
        let octets = host.split(separator: ".")
        guard octets.count == 4,
              let values = octets.map({ UInt8($0) }).nilIfAnyNil else { return false }
        return values[0] == 10 || (values[0] == 172 && (16...31).contains(values[1])) ||
            (values[0] == 192 && values[1] == 168)
    }

    private static func isHostnameOrIPv4(_ host: String) -> Bool {
        guard !host.isEmpty, !host.contains(":"), !host.hasPrefix("."), !host.hasSuffix(".") else { return false }
        let labels = host.split(separator: ".", omittingEmptySubsequences: false)
        guard labels.count >= 2 else { return false }
        return labels.allSatisfy { label in
            guard let first = label.first, let last = label.last,
                  first.isLetter || first.isNumber, last.isLetter || last.isNumber else { return false }
            return label.allSatisfy { $0.isLetter || $0.isNumber || $0 == "-" }
        }
    }
}

private extension Array where Element == UInt8? {
    var nilIfAnyNil: [UInt8]? {
        guard allSatisfy({ $0 != nil }) else { return nil }
        return compactMap { $0 }
    }
}

private final class MonitterWebView: WKWebView {
    // Monitter's composer already owns its input controls. Keep the system
    // keyboard, but omit WebKit's separate previous/next/done form toolbar.
    override var inputAccessoryView: UIView? { nil }
}

/// Keeps a persistent WebKit page and asks its already-paired transport to
/// resume after iOS foreground or network-path changes. It deliberately never
/// reloads the page: the browser session owns reconnect/backoff and persistent
/// pairing keys.
@MainActor
private final class WebConnectionRecovery: ObservableObject {
    private let pathMonitor = NWPathMonitor()
    private let pathQueue = DispatchQueue(label: "com.soyrex.monitter.network-path")
    private weak var webView: WKWebView?
    private var trustedURL: URL?
    private var pendingResume: DispatchWorkItem?
    private var hasObservedPath = false

    init() {
        pathMonitor.pathUpdateHandler = { [weak self] path in
            let status = path.status
            DispatchQueue.main.async { self?.networkPathChanged(status: status) }
        }
        pathMonitor.start(queue: pathQueue)
    }

    deinit {
        pendingResume?.cancel()
        pathMonitor.cancel()
    }

    func setWebView(_ webView: WKWebView) {
        self.webView = webView
    }

    func setTrustedURL(_ url: URL?) {
        trustedURL = url
    }

    func resumeAfterForeground() {
        queueResume()
    }

    func deliverPairingInvitation(_ invitation: String) {
        guard let trustedURL, isPairedLoopback(trustedURL), let webView,
              matchesTrustedOrigin(webView.url, trustedURL),
              let data = try? JSONEncoder().encode(invitation),
              let value = String(data: data, encoding: .utf8) else { return }
        webView.evaluateJavaScript("window.dispatchEvent(new CustomEvent('monitter:qr', { detail: \(value) }));") { _, error in
            if let error {
                NSLog("Monitter QR delivery failed: %@", error.localizedDescription)
            } else {
                NSLog("Monitter QR invitation delivered to paired web view")
            }
        }
    }

    private func networkPathChanged(status: NWPath.Status) {
        // NWPathMonitor reports its initial state immediately. It is not a
        // recovery, and reacting to it would duplicate the launch connection.
        // Two usable Wi-Fi paths can have different routes/DNS while reporting
        // the same interface type. Do not discard such path changes.
        defer { hasObservedPath = true }
        guard hasObservedPath, status == .satisfied else { return }
        queueResume()
    }

    private func queueResume() {
        guard trustedURL != nil else { return }
        pendingResume?.cancel()
        let work = DispatchWorkItem { [weak self] in self?.dispatchResume() }
        pendingResume = work
        // UIApplication and NWPathMonitor commonly report the same recovery
        // together. Coalescing here leaves one reconnect request for WebKit.
        DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(300), execute: work)
    }

    private func dispatchResume() {
        pendingResume = nil
        guard let trustedURL, isPairedLoopback(trustedURL), let webView, matchesTrustedOrigin(webView.url, trustedURL) else { return }
        webView.evaluateJavaScript("window.dispatchEvent(new CustomEvent('monitter:resume'))")
    }

    private func matchesTrustedOrigin(_ pageURL: URL?, _ trustedURL: URL) -> Bool {
        guard let pageURL else { return false }
        return pageURL.scheme?.lowercased() == trustedURL.scheme?.lowercased()
            && pageURL.host?.lowercased() == trustedURL.host?.lowercased()
            && pageURL.port == trustedURL.port
    }

    private func isPairedLoopback(_ url: URL) -> Bool {
        url.scheme?.lowercased() == "http" && url.host == "127.0.0.1" && url.port == 31977
    }
}

private struct WebContentView: UIViewRepresentable {
    @Binding var webView: WKWebView?
    let url: URL?
    let error: String?
    let webViewReady: (WKWebView) -> Void
    let requestScan: () -> Void
    let requestConnectionChooser: () -> Void
    let navigationError: (String) -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(requestScan: requestScan, requestConnectionChooser: requestConnectionChooser, navigationError: navigationError)
    }

    func makeUIView(context: Context) -> WKWebView {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .default()
        configuration.defaultWebpagePreferences.allowsContentJavaScript = true
        configuration.userContentController.add(context.coordinator, name: "scanQR")
        configuration.userContentController.add(context.coordinator, name: "changeConnection")
        configuration.userContentController.addScriptMessageHandler(PairingKeychainBridge(), contentWorld: .page, name: "monitterPairing")
        configuration.userContentController.add(context.coordinator, name: "monitterSpeech")
        configuration.userContentController.add(context.coordinator, name: "monitterCompanion")
        let view = MonitterWebView(frame: .zero, configuration: configuration)
        view.allowsBackForwardNavigationGestures = true
        view.navigationDelegate = context.coordinator
        view.uiDelegate = context.coordinator
        view.isOpaque = false
        view.backgroundColor = .systemBackground
        webView = view
        context.coordinator.speechWebView = view
        webViewReady(view)
        return view
    }

    func updateUIView(_ uiView: WKWebView, context: Context) {
        context.coordinator.pairedMode = url?.host == "127.0.0.1"
        context.coordinator.speechWebView = uiView
        // SwiftUI can redraw while the paired page navigates from /mobile to
        // the owner workspace at /. Reload only when the chosen connection
        // changes, not whenever WebKit's current page differs from that entry.
        if let url, context.coordinator.requestedURL != url {
            context.coordinator.requestedURL = url
            uiView.load(URLRequest(url: url))
        } else if url == nil {
            context.coordinator.requestedURL = nil
        }
        if let error, url == nil, uiView.url == nil {
            let escaped = error.replacingOccurrences(of: "&", with: "&amp;").replacingOccurrences(of: "<", with: "&lt;")
            uiView.loadHTMLString("<main><h1>Monitter unavailable</h1><p>\(escaped)</p></main>", baseURL: nil)
        }
    }

    static func dismantleUIView(_ uiView: WKWebView, coordinator: Coordinator) {
        uiView.configuration.userContentController.removeScriptMessageHandler(forName: "monitterPairing")
        uiView.configuration.userContentController.removeScriptMessageHandler(forName: "monitterSpeech")
        uiView.configuration.userContentController.removeScriptMessageHandler(forName: "monitterCompanion")
        coordinator.cancelSpeech()
        coordinator.teardownCompanion()
    }

    final class Coordinator: NSObject, WKScriptMessageHandler, WKNavigationDelegate, WKUIDelegate {
        private let requestScan: () -> Void
        private let requestConnectionChooser: () -> Void
        private let navigationError: (String) -> Void
        var pairedMode = false
        var requestedURL: URL?
        private var speechEngine: AVAudioEngine?
        private var speechRequest: SFSpeechAudioBufferRecognitionRequest?
        private var recognitionTask: SFSpeechRecognitionTask?
        private var speechSessionId: String?
        private var speechDeadline: DispatchWorkItem?
        private var speechFinalizationWatchdog: DispatchWorkItem?
        private var lastPartialTranscript: String?
        private var companionActive = false
        private var appIsForeground = UIApplication.shared.applicationState == .active
        private var idleTimerPriorValue: Bool?
        private var lifecycleObservers: [NSObjectProtocol] = []
        weak var speechWebView: WKWebView?
        init(requestScan: @escaping () -> Void, requestConnectionChooser: @escaping () -> Void, navigationError: @escaping (String) -> Void) {
            self.requestScan = requestScan
            self.requestConnectionChooser = requestConnectionChooser
            self.navigationError = navigationError
            super.init()
            let center = NotificationCenter.default
            lifecycleObservers.append(center.addObserver(forName: UIApplication.willResignActiveNotification, object: nil, queue: .main) { [weak self] _ in
                guard let self else { return }
                self.appIsForeground = false
                self.syncIdleTimer()
                // Permission prompts briefly resign active too. An authorization
                // request may finish when the prompt closes, but active capture stops.
                if self.speechEngine != nil {
                    self.stopSpeechForLifecycle("Speech stopped because Monitter left the foreground.")
                }
            })
            lifecycleObservers.append(center.addObserver(forName: UIApplication.didEnterBackgroundNotification, object: nil, queue: .main) { [weak self] _ in
                self?.stopSpeechForLifecycle("Speech stopped because Monitter entered the background.")
            })
            lifecycleObservers.append(center.addObserver(forName: UIApplication.didBecomeActiveNotification, object: nil, queue: .main) { [weak self] _ in
                guard let self else { return }
                self.appIsForeground = true
                self.syncIdleTimer()
            })
            lifecycleObservers.append(center.addObserver(forName: AVAudioSession.interruptionNotification, object: AVAudioSession.sharedInstance(), queue: .main) { [weak self] notification in
                guard let self,
                      let rawType = notification.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
                      AVAudioSession.InterruptionType(rawValue: rawType) == .began else { return }
                self.stopSpeechForLifecycle("Speech stopped because audio was interrupted.")
            })
            lifecycleObservers.append(center.addObserver(forName: AVAudioSession.routeChangeNotification, object: AVAudioSession.sharedInstance(), queue: .main) { [weak self] notification in
                guard let self,
                      let rawReason = notification.userInfo?[AVAudioSessionRouteChangeReasonKey] as? UInt,
                      AVAudioSession.RouteChangeReason(rawValue: rawReason) == .oldDeviceUnavailable else { return }
                self.stopSpeechForLifecycle("Speech stopped because the audio route changed.")
            })
        }
        deinit {
            lifecycleObservers.forEach(NotificationCenter.default.removeObserver)
            if idleTimerPriorValue != nil {
                UIApplication.shared.isIdleTimerDisabled = idleTimerPriorValue ?? false
            }
        }
        private func syncIdleTimer() {
            let shouldKeepAwake = companionActive && appIsForeground
            if shouldKeepAwake {
                if idleTimerPriorValue == nil {
                    idleTimerPriorValue = UIApplication.shared.isIdleTimerDisabled
                }
                UIApplication.shared.isIdleTimerDisabled = true
            } else if let priorValue = idleTimerPriorValue {
                UIApplication.shared.isIdleTimerDisabled = priorValue
                idleTimerPriorValue = nil
            }
        }
        private func stopSpeechForLifecycle(_ reason: String) {
            guard let sessionId = speechSessionId else { return }
            finishSpeech(teardown: false)
            emit(sessionId: sessionId, type: "error", error: reason)
        }
        func teardownCompanion() {
            companionActive = false
            appIsForeground = false
            syncIdleTimer()
            stopSpeechForLifecycle("Speech stopped because the paired page closed.")
            lifecycleObservers.forEach(NotificationCenter.default.removeObserver)
            lifecycleObservers.removeAll()
        }
        private func handleCompanion(_ body: [String: Any]) {
            guard pairedMode,
                  requestedURL != nil,
                  let webView = speechWebView,
                  let pageURL = webView.url,
                  pageURL.scheme == "http", pageURL.host == "127.0.0.1", pageURL.port == 31977,
                  body.count == 1,
                  let active = body["active"] as? Bool else { return }
            companionActive = active
            if !active {
                stopSpeechForLifecycle("Speech stopped because companion mode ended.")
            }
            syncIdleTimer()
        }
        func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
            failed(error)
        }
        func webView(_ webView: WKWebView, didStartProvisionalNavigation navigation: WKNavigation!) {
            companionActive = false
            syncIdleTimer()
            stopSpeechForLifecycle("Speech stopped because the paired page navigated.")
        }
        func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
            failed(error)
        }
        func webView(
            _ webView: WKWebView,
            requestMediaCapturePermissionFor origin: WKSecurityOrigin,
            initiatedByFrame frame: WKFrameInfo,
            type: WKMediaCaptureType,
            decisionHandler: @escaping (WKPermissionDecision) -> Void
        ) {
            // The bundled paired workspace is the only HTTP origin that may
            // request capture. LAN workspaces have no implicit media trust.
            let isBundledPairedMainFrame = pairedMode
                && frame.isMainFrame
                && origin.protocol == "http"
                && origin.host == "127.0.0.1"
                && origin.port == 31977
            decisionHandler(isBundledPairedMainFrame && type == .microphone ? .grant : .deny)
        }
        private func failed(_ error: Error) {
            let nsError = error as NSError
            guard nsError.code != NSURLErrorCancelled else { return }
            DispatchQueue.main.async { self.navigationError(error.localizedDescription) }
        }
        func userContentController(_ userContentController: WKUserContentController, didReceive message: WKScriptMessage) {
            if message.name == "monitterCompanion" {
                guard pairedMode,
                      requestedURL != nil,
                      message.frameInfo.isMainFrame,
                      message.frameInfo.securityOrigin.protocol == "http",
                      message.frameInfo.securityOrigin.host == "127.0.0.1",
                      message.frameInfo.securityOrigin.port == 31977,
                      let body = message.body as? [String: Any] else { return }
                handleCompanion(body)
                return
            }
            if message.name == "monitterSpeech" {
                guard message.frameInfo.isMainFrame,
                      message.frameInfo.securityOrigin.protocol == "http",
                      message.frameInfo.securityOrigin.host == "127.0.0.1",
                      message.frameInfo.securityOrigin.port == 31977,
                      let body = message.body as? [String: Any] else { return }
                handleSpeech(body)
                return
            }
            if message.name == "changeConnection" {
                guard message.frameInfo.isMainFrame,
                      requestedURL != nil,
                      let pageURL = message.webView?.url,
                      message.frameInfo.securityOrigin.host == pageURL.host,
                      message.frameInfo.securityOrigin.protocol == pageURL.scheme else { return }
                requestConnectionChooser()
                return
            }
            guard pairedMode, message.name == "scanQR", message.frameInfo.isMainFrame,
                  message.frameInfo.securityOrigin.protocol == "http",
                  message.frameInfo.securityOrigin.host == "127.0.0.1",
                  message.frameInfo.securityOrigin.port == 31977 else { return }
            requestScan()
        }

        func cancelSpeech() {
            finishSpeech(teardown: true)
        }

        private func finishSpeech(teardown: Bool) {
            speechDeadline?.cancel()
            speechDeadline = nil
            speechFinalizationWatchdog?.cancel()
            speechFinalizationWatchdog = nil
            speechEngine?.inputNode.removeTap(onBus: 0)
            speechEngine?.stop()
            try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
            speechRequest?.endAudio()
            recognitionTask?.cancel()
            recognitionTask = nil
            speechRequest = nil
            speechEngine = nil
            let oldSession = speechSessionId
            speechSessionId = nil
            lastPartialTranscript = nil
            if teardown, let oldSession { emit(sessionId: oldSession, type: "stopped") }
        }

        private func emit(sessionId: String, type: String, available: Bool? = nil, text: String? = nil, error: String? = nil, level: Double? = nil) {
            guard let webView = speechWebView, pairedMode,
                  let page = webView.url, page.scheme == "http", page.host == "127.0.0.1", page.port == 31977 else { return }
            var detail: [String: Any] = ["sessionId": sessionId, "type": type]
            if let available { detail["available"] = available }
            if let text { detail["text"] = text }
            if let error { detail["error"] = error }
            if let level { detail["level"] = level }
            guard let data = try? JSONSerialization.data(withJSONObject: detail, options: [.fragmentsAllowed]),
                  var json = String(data: data, encoding: .utf8) else { return }
            json = json.replacingOccurrences(of: "\\u2028", with: "\\\\u2028")
                .replacingOccurrences(of: "\\u2029", with: "\\\\u2029")
            webView.evaluateJavaScript("window.dispatchEvent(new CustomEvent('monitter:native-speech', {detail:\(json)}))")
        }

        private func speechRecognizer(localeIdentifier: String?) -> SFSpeechRecognizer? {
            let locale = localeIdentifier.flatMap(Locale.init(identifier:)) ?? Locale.current
            guard let recognizer = SFSpeechRecognizer(locale: locale), recognizer.isAvailable,
                  recognizer.supportsOnDeviceRecognition else { return nil }
            return recognizer
        }

        private func handleSpeech(_ body: [String: Any]) {
            guard pairedMode,
                  requestedURL != nil,
                  let webView = speechWebView,
                  let pageURL = webView.url,
                  pageURL.scheme == "http", pageURL.host == "127.0.0.1", pageURL.port == 31977 else { return }
            guard let action = body["action"] as? String,
                  let sessionId = body["sessionId"] as? String,
                  !sessionId.isEmpty, sessionId.count <= 128 else { return }
            let localeIdentifier = body["locale"] as? String
            switch action {
            case "status":
                emit(sessionId: sessionId, type: "availability", available: speechRecognizer(localeIdentifier: localeIdentifier) != nil)
            case "stop":
                guard speechSessionId == sessionId else { return }
                endCaptureAndScheduleFinalization(sessionId: sessionId)
            case "cancel":
                guard speechSessionId == sessionId else { return }
                finishSpeech(teardown: true)
            case "start":
                startSpeech(sessionId: sessionId, localeIdentifier: localeIdentifier)
            default:
                return
            }
        }

        private func startSpeech(sessionId: String, localeIdentifier: String?) {
            guard speechSessionId == nil else {
                emit(sessionId: sessionId, type: "error", error: "A speech session is already active.")
                return
            }
            guard let recognizer = speechRecognizer(localeIdentifier: localeIdentifier) else {
                emit(sessionId: sessionId, type: "error", error: "On-device speech recognition is unavailable for this locale.")
                return
            }
            speechSessionId = sessionId
            lastPartialTranscript = nil
            requestSpeechAuthorization(sessionId: sessionId, recognizer: recognizer)
        }

        private func endCaptureAndScheduleFinalization(sessionId: String) {
            guard speechSessionId == sessionId else { return }
            speechDeadline?.cancel()
            speechDeadline = nil
            speechEngine?.stop()
            speechRequest?.endAudio()
            speechFinalizationWatchdog?.cancel()
            let watchdog = DispatchWorkItem { [weak self] in
                guard let self, self.speechSessionId == sessionId else { return }
                if let transcript = self.lastPartialTranscript, !transcript.isEmpty {
                    self.emit(sessionId: sessionId, type: "final", text: transcript)
                } else {
                    self.emit(sessionId: sessionId, type: "error", error: "Speech recognition did not return a final transcript.")
                }
                self.finishSpeech(teardown: false)
            }
            speechFinalizationWatchdog = watchdog
            DispatchQueue.main.asyncAfter(deadline: .now() + 8, execute: watchdog)
        }

        private func requestSpeechAuthorization(sessionId: String, recognizer: SFSpeechRecognizer) {
            let requestMicrophone: (@escaping (Bool) -> Void) -> Void = { completion in
                if #available(iOS 17.0, *) {
                    AVAudioApplication.requestRecordPermission(completionHandler: completion)
                } else {
                    AVAudioSession.sharedInstance().requestRecordPermission(completion)
                }
            }
            let begin: () -> Void = { [weak self] in
                requestMicrophone { granted in
                    DispatchQueue.main.async {
                        guard let self, self.speechSessionId == sessionId else { return }
                        guard granted else {
                            self.finishSpeech(teardown: false)
                            self.emit(sessionId: sessionId, type: "error", error: "Microphone access was denied.")
                            return
                        }
                        self.beginRecognition(sessionId: sessionId, recognizer: recognizer)
                    }
                }
            }
            switch SFSpeechRecognizer.authorizationStatus() {
            case .authorized: begin()
            case .notDetermined:
                SFSpeechRecognizer.requestAuthorization { status in
                    DispatchQueue.main.async {
                        guard self.speechSessionId == sessionId else { return }
                        guard status == .authorized else {
                            self.finishSpeech(teardown: false)
                            self.emit(sessionId: sessionId, type: "error", error: "Speech recognition access was denied.")
                            return
                        }
                        begin()
                    }
                }
            default:
                finishSpeech(teardown: false)
                emit(sessionId: sessionId, type: "error", error: "Speech recognition access is unavailable or denied.")
            }
        }

        private func beginRecognition(sessionId: String, recognizer: SFSpeechRecognizer, foregroundAttempt: Int = 0) {
            guard speechSessionId == sessionId else { return }
            guard appIsForeground else {
                // Authorization completion may precede didBecomeActive by one run loop.
                // Never start audio in the background or revive a cancelled request.
                if foregroundAttempt < 20 {
                    DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) { [weak self] in
                        self?.beginRecognition(sessionId: sessionId, recognizer: recognizer, foregroundAttempt: foregroundAttempt + 1)
                    }
                } else {
                    stopSpeechForLifecycle("Return to Monitter and tap the microphone to start listening.")
                }
                return
            }
            let engine = AVAudioEngine()
            let request = SFSpeechAudioBufferRecognitionRequest()
            request.shouldReportPartialResults = true
            request.requiresOnDeviceRecognition = true
            request.contextualStrings = ["Hey Mo", "Mo"]
            do {
                let audioSession = AVAudioSession.sharedInstance()
                try audioSession.setCategory(.record, mode: .measurement, options: [.duckOthers])
                try audioSession.setActive(true, options: .notifyOthersOnDeactivation)
                let input = engine.inputNode
                let format = input.outputFormat(forBus: 0)
                var lastLevelSent = 0.0
                input.installTap(onBus: 0, bufferSize: 1024, format: format) { buffer, _ in
                    request.append(buffer)
                    let now = ProcessInfo.processInfo.systemUptime
                    guard now - lastLevelSent >= 0.06,
                          let samples = buffer.floatChannelData?[0], buffer.frameLength > 0 else { return }
                    lastLevelSent = now
                    var sum: Float = 0
                    for index in 0..<Int(buffer.frameLength) { sum += samples[index] * samples[index] }
                    let rms = sqrt(sum / Float(buffer.frameLength))
                    let decibels = 20 * log10(max(rms, 0.00001))
                    let level = Double(max(0, min(1, (decibels + 48) / 36)))
                    DispatchQueue.main.async { [weak self] in
                        guard let self, self.speechSessionId == sessionId else { return }
                        self.emit(sessionId: sessionId, type: "level", level: level)
                    }
                }
                engine.prepare()
                try engine.start()
                speechEngine = engine
                speechRequest = request
                emit(sessionId: sessionId, type: "started")
                recognitionTask = recognizer.recognitionTask(with: request) { [weak self] result, error in
                    DispatchQueue.main.async {
                        guard let self, self.speechSessionId == sessionId else { return }
                        if let result {
                            let text = result.bestTranscription.formattedString
                            if !result.isFinal, !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                                self.lastPartialTranscript = text
                            }
                            self.emit(sessionId: sessionId, type: result.isFinal ? "final" : "partial", text: text)
                            if result.isFinal {
                                self.finishSpeech(teardown: false)
                            }
                        }
                        else if let error {
                            self.finishSpeech(teardown: false)
                            self.emit(sessionId: sessionId, type: "error", error: error.localizedDescription)
                        }
                    }
                }
                let deadline = DispatchWorkItem { [weak self] in
                    guard let self, self.speechSessionId == sessionId else { return }
                    self.endCaptureAndScheduleFinalization(sessionId: sessionId)
                }
                speechDeadline = deadline
                DispatchQueue.main.asyncAfter(deadline: .now() + 90, execute: deadline)
            } catch {
                finishSpeech(teardown: false)
                emit(sessionId: sessionId, type: "error", error: error.localizedDescription)
            }
        }
    }
}


/// The bundled, main-frame pairing page is the only caller. Pairing identity and
/// locator survive WebKit storage eviction and app updates in device-only Keychain.
private final class PairingKeychainBridge: NSObject, WKScriptMessageHandlerWithReply {
    private struct Record: Codable {
        var privateKey: Data
        var invitation: String?
        var savedAt: Double?
    }
    private let query: [String: Any] = [
        kSecClass as String: kSecClassGenericPassword,
        kSecAttrService as String: "com.soyrex.monitter.ios.pairing.v1",
        kSecAttrAccount as String: "desktop-pairing"
    ]
    private func load() throws -> Record? {
        var request = query
        request[kSecReturnData as String] = true
        request[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        let status = SecItemCopyMatching(request as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess, let data = result as? Data else {
            throw NSError(domain: "MonitterPairing", code: Int(status), userInfo: [NSLocalizedDescriptionKey: "Unlock your iPhone to read its saved pairing."])
        }
        return try JSONDecoder().decode(Record.self, from: data)
    }
    private func save(_ record: Record) throws {
        let data = try JSONEncoder().encode(record)
        let attributes: [String: Any] = [kSecValueData as String: data,
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly]
        var status = SecItemUpdate(query as CFDictionary, attributes as CFDictionary)
        if status == errSecItemNotFound {
            var insert = query
            attributes.forEach { insert[$0.key] = $0.value }
            status = SecItemAdd(insert as CFDictionary, nil)
        }
        guard status == errSecSuccess else {
            throw NSError(domain: "MonitterPairing", code: Int(status), userInfo: [NSLocalizedDescriptionKey: "Could not securely remember this desktop. Retry before closing the app."])
        }
    }
    func userContentController(_ userContentController: WKUserContentController,
                               didReceive message: WKScriptMessage,
                               replyHandler: @escaping (Any?, String?) -> Void) {
        let origin = message.frameInfo.securityOrigin
        guard message.frameInfo.isMainFrame, origin.protocol == "http",
              origin.host == "127.0.0.1", origin.port == 31977,
              let body = message.body as? [String: Any], let action = body["action"] as? String else {
            replyHandler(nil, "Pairing storage is available only to the bundled Monitter app.")
            return
        }
        do {
            var record = try load()
            switch action {
            case "identity":
                if record == nil {
                    record = Record(privateKey: P256.KeyAgreement.PrivateKey().derRepresentation)
                    try save(record!)
                }
                let key = try P256.KeyAgreement.PrivateKey(derRepresentation: record!.privateKey)
                replyHandler(["privateKeyPkcs8": key.derRepresentation.base64EncodedString(),
                              "publicKeyRaw": key.publicKey.x963Representation.base64EncodedString()], nil)
            case "load":
                if let record, let invitation = record.invitation, let savedAt = record.savedAt {
                    replyHandler(["invitation": invitation, "savedAt": savedAt], nil)
                } else { replyHandler(NSNull(), nil) }
            case "save":
                guard var existing = record, let invitation = body["invitation"] as? String,
                      invitation.utf8.count <= 8192,
                      let data = invitation.data(using: .utf8),
                      let value = try JSONSerialization.jsonObject(with: data) as? [String: Any],
                      value["version"] as? Int == 2,
                      value["publicKey"] is String, value["room"] is String,
                      let savedAt = body["savedAt"] as? Double, savedAt.isFinite else {
                    replyHandler(nil, "Pairing record is invalid or device identity is missing.")
                    return
                }
                existing.invitation = invitation
                existing.savedAt = savedAt
                try save(existing)
                replyHandler(true, nil)
            case "forget":
                if var existing = record {
                    existing.invitation = nil
                    existing.savedAt = nil
                    try save(existing)
                }
                replyHandler(true, nil)
            default: replyHandler(nil, "Unknown pairing storage action.")
            }
        } catch { replyHandler(nil, error.localizedDescription) }
    }
}
