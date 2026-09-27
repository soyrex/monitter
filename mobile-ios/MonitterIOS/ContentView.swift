import SwiftUI
import WebKit

private enum ConnectionMode: Equatable {
    case chooser
    case paired
    case lan(URL)
}

struct ContentView: View {
    @AppStorage("ownerLANBaseURL") private var savedLANAddress = ""
    @AppStorage("lastConnectionMode") private var savedMode = ""
    @State private var mode: ConnectionMode = .chooser
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
                ZStack(alignment: .topTrailing) {
                    WebContentView(webView: $webView, url: webURL, error: startError, requestScan: {
                        scannerPresented = true
                    }, navigationError: { pageError = $0 })
                    .ignoresSafeArea()

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

                    Button {
                        editingAddress = true
                    } label: {
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
        }
        .task {
            if savedMode == "lan", !savedLANAddress.isEmpty, let url = LANAddress.parse(savedLANAddress) {
                mode = .lan(url)
                webURL = url
            } else if savedMode == "paired" {
                mode = .paired
            }
            startServerIfNeeded()
        }
        .sheet(isPresented: $editingAddress) { connectionChooser }
        .fullScreenCover(isPresented: $scannerPresented) {
            QRScannerView(onScan: { invitation in
                scannerPresented = false
                deliver(invitation: invitation)
            }, onCancel: {
                scannerPresented = false
            })
        }
    }

    private var connectionChooser: some View {
        NavigationStack {
            Form {
                Section("Owner workspace") {
                    TextField("http://192.168.1.20:18436", text: $address)
                        .keyboardType(.URL)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    Text("Enter the Monitter desktop address on your local network. HTTP is accepted for private IPv4 and .local addresses; HTTPS requires port 18436.")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                    Button("Connect to owner workspace") { connectToLAN() }
                        .disabled(LANAddress.parse(address) == nil)
                }
                Section("Owner workspace via Monitter relay") {
                    Text("Use the same responsive workspace away from your local network. Your desktop must separately approve full-workspace access for this phone.")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                    Button("Connect through relay") { usePairedMode() }
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

    private func deliver(invitation: String) {
        guard case .paired = mode,
              let data = try? JSONEncoder().encode(invitation),
              let value = String(data: data, encoding: .utf8) else { return }
        webView?.evaluateJavaScript("window.dispatchEvent(new CustomEvent('monitter:qr', { detail: \(value) }));")
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

private struct WebContentView: UIViewRepresentable {
    @Binding var webView: WKWebView?
    let url: URL?
    let error: String?
    let requestScan: () -> Void
    let navigationError: (String) -> Void

    func makeCoordinator() -> Coordinator { Coordinator(requestScan: requestScan, navigationError: navigationError) }

    func makeUIView(context: Context) -> WKWebView {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .default()
        configuration.defaultWebpagePreferences.allowsContentJavaScript = true
        configuration.userContentController.add(context.coordinator, name: "scanQR")
        let view = WKWebView(frame: .zero, configuration: configuration)
        view.allowsBackForwardNavigationGestures = true
        view.navigationDelegate = context.coordinator
        view.isOpaque = false
        view.backgroundColor = .systemBackground
        webView = view
        return view
    }

    func updateUIView(_ uiView: WKWebView, context: Context) {
        context.coordinator.pairedMode = url?.host == "127.0.0.1"
        // The paired bootstrap page navigates to the full owner workspace on
        // this same private loopback origin after explicit desktop approval.
        // A SwiftUI redraw must not send it back to the pairing screen.
        let inOwnerWorkspace = url?.host == "127.0.0.1"
            && url?.port == 31977
            && uiView.url?.host == "127.0.0.1"
            && uiView.url?.port == 31977
            && uiView.url?.path == "/"
            && uiView.url.flatMap { URLComponents(url: $0, resolvingAgainstBaseURL: false) }?.queryItems?.contains(where: { $0.name == "monitter-owner-relay" && $0.value == "1" }) == true
        if let url, uiView.url?.absoluteString != url.absoluteString && !inOwnerWorkspace { uiView.load(URLRequest(url: url)) }
        else if let error, uiView.url == nil {
            let escaped = error.replacingOccurrences(of: "&", with: "&amp;").replacingOccurrences(of: "<", with: "&lt;")
            uiView.loadHTMLString("<main><h1>Monitter unavailable</h1><p>\(escaped)</p></main>", baseURL: nil)
        }
    }

    final class Coordinator: NSObject, WKScriptMessageHandler, WKNavigationDelegate {
        private let requestScan: () -> Void
        private let navigationError: (String) -> Void
        var pairedMode = false
        init(requestScan: @escaping () -> Void, navigationError: @escaping (String) -> Void) {
            self.requestScan = requestScan
            self.navigationError = navigationError
        }
        func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
            failed(error)
        }
        func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
            failed(error)
        }
        private func failed(_ error: Error) {
            let nsError = error as NSError
            guard nsError.code != NSURLErrorCancelled else { return }
            DispatchQueue.main.async { self.navigationError(error.localizedDescription) }
        }
        func userContentController(_ userContentController: WKUserContentController, didReceive message: WKScriptMessage) {
            guard pairedMode, message.name == "scanQR", message.frameInfo.isMainFrame,
                  message.frameInfo.securityOrigin.protocol == "http",
                  message.frameInfo.securityOrigin.host == "127.0.0.1",
                  message.frameInfo.securityOrigin.port == 31977 else { return }
            requestScan()
        }
    }
}
