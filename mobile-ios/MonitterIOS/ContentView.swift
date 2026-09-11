import SwiftUI
import WebKit

struct ContentView: View {
    @State private var scannerPresented = false
    @State private var webView: WKWebView?
    @State private var webURL: URL?
    @State private var server: BundleWebServer?
    @State private var startError: String?

    var body: some View {
        WebContentView(webView: $webView, url: webURL, error: startError, requestScan: {
            scannerPresented = true
        })
        .ignoresSafeArea(edges: .bottom)
        .task {
            startServerIfNeeded()
        }
        .fullScreenCover(isPresented: $scannerPresented) {
            QRScannerView(onScan: { invitation in
                scannerPresented = false
                deliver(invitation: invitation)
            }, onCancel: {
                scannerPresented = false
            })
        }
    }

    private func deliver(invitation: String) {
        guard let data = try? JSONEncoder().encode(invitation),
              let value = String(data: data, encoding: .utf8) else {
            return
        }

        webView?.evaluateJavaScript(
            "window.dispatchEvent(new CustomEvent('monitter:qr', { detail: \(value) }));"
        )
    }

    private func startServerIfNeeded() {
        guard server == nil else { return }
        guard let webRoot = Bundle.main.url(forResource: "Web", withExtension: nil) else {
            startError = "Monitter mobile build missing. Run mobile-ios/build.sh before building the app."
            return
        }

        do {
            let newServer = try BundleWebServer(root: webRoot)
            server = newServer
            newServer.start(
                onReady: { url in webURL = url },
                onFailure: { message in startError = message }
            )
        } catch {
            startError = "Unable to start the local Monitter server: \(error.localizedDescription)"
        }
    }
}

private struct WebContentView: UIViewRepresentable {
    @Binding var webView: WKWebView?
    let url: URL?
    let error: String?
    let requestScan: () -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(requestScan: requestScan)
    }

    func makeUIView(context: Context) -> WKWebView {
        let configuration = WKWebViewConfiguration()
        configuration.defaultWebpagePreferences.allowsContentJavaScript = true
        configuration.userContentController.add(context.coordinator, name: "scanQR")

        let view = WKWebView(frame: .zero, configuration: configuration)
        view.allowsBackForwardNavigationGestures = true
        view.isOpaque = false
        view.backgroundColor = .systemBackground
        webView = view
        return view
    }

    func updateUIView(_ uiView: WKWebView, context: Context) {
        if let url, uiView.url != url {
            uiView.load(URLRequest(url: url))
        } else if let error, uiView.url == nil {
            uiView.loadHTMLString("<main><h1>Monitter unavailable</h1><p>\(error)</p></main>", baseURL: nil)
        }
    }

    final class Coordinator: NSObject, WKScriptMessageHandler {
        private let requestScan: () -> Void

        init(requestScan: @escaping () -> Void) {
            self.requestScan = requestScan
        }

        func userContentController(_ userContentController: WKUserContentController, didReceive message: WKScriptMessage) {
            guard message.name == "scanQR",
                  message.frameInfo.isMainFrame,
                  message.frameInfo.securityOrigin.protocol == "http",
                  message.frameInfo.securityOrigin.host == "127.0.0.1" else { return }
            requestScan()
        }
    }
}
