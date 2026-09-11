import Foundation
import Network

/// A private asset server for the bundled Svelte SPA. It binds only to the
/// loopback interface and permits GET requests for regular files below `root`.
final class BundleWebServer {
    private let root: URL
    private let listener: NWListener
    private let queue = DispatchQueue(label: "com.soyrex.monitter.web-server")

    init(root: URL) throws {
        self.root = root.standardizedFileURL
        let parameters = NWParameters.tcp
        parameters.requiredLocalEndpoint = .hostPort(
            host: .ipv4(IPv4Address("127.0.0.1")!),
            port: .any
        )
        listener = try NWListener(using: parameters)
        listener.newConnectionHandler = { [weak self] connection in
            self?.handle(connection)
        }
    }

    func start(onReady: @escaping (URL) -> Void, onFailure: @escaping (String) -> Void) {
        listener.stateUpdateHandler = { [weak self] state in
            guard let self else { return }
            switch state {
            case .ready:
                guard let port = self.listener.port,
                      let url = URL(string: "http://127.0.0.1:\(port)/mobile") else {
                    DispatchQueue.main.async { onFailure("The local server did not provide a port.") }
                    return
                }
                DispatchQueue.main.async { onReady(url) }
            case .failed(let error):
                DispatchQueue.main.async { onFailure(error.localizedDescription) }
            default:
                break
            }
        }
        listener.start(queue: queue)
    }

    deinit {
        listener.cancel()
    }

    private func handle(_ connection: NWConnection) {
        connection.start(queue: queue)
        let timeout = DispatchWorkItem { connection.cancel() }
        queue.asyncAfter(deadline: .now() + 5, execute: timeout)
        receiveRequest(connection, buffer: Data(), timeout: timeout)
    }

    private func receiveRequest(_ connection: NWConnection, buffer: Data, timeout: DispatchWorkItem) {
        connection.receive(minimumIncompleteLength: 1, maximumLength: 4_096) { [weak self] data, _, isComplete, _ in
            guard let self, let data else {
                timeout.cancel()
                connection.cancel()
                return
            }

            var request = buffer
            request.append(data)
            guard request.count <= 8_192 else {
                timeout.cancel()
                self.send(status: "431 Request Header Fields Too Large", body: Data(), contentType: "text/plain", connection: connection)
                return
            }

            if let range = request.range(of: Data("\r\n\r\n".utf8)) {
                timeout.cancel()
                guard let header = String(data: request[..<range.upperBound], encoding: .utf8) else {
                    connection.cancel()
                    return
                }
                self.respond(to: header, connection: connection)
            } else if isComplete {
                timeout.cancel()
                connection.cancel()
            } else {
                self.receiveRequest(connection, buffer: request, timeout: timeout)
            }
        }
    }

    private func respond(to request: String, connection: NWConnection) {
        let parts = request.split(separator: "\r\n", maxSplits: 1).first?.split(separator: " ") ?? []
        guard parts.count >= 2, parts[0] == "GET" else {
            send(status: "405 Method Not Allowed", body: Data(), contentType: "text/plain", connection: connection)
            return
        }

        let requestedPath = String(parts[1].split(separator: "?", maxSplits: 1).first ?? "")
        guard let decodedPath = requestedPath.removingPercentEncoding,
              let fileURL = fileURL(for: decodedPath) else {
            send(status: "404 Not Found", body: Data(), contentType: "text/plain", connection: connection)
            return
        }

        do {
            let values = try fileURL.resourceValues(forKeys: [.isRegularFileKey])
            guard values.isRegularFile == true else { throw CocoaError(.fileNoSuchFile) }
            send(status: "200 OK", body: try Data(contentsOf: fileURL), contentType: mimeType(for: fileURL), connection: connection)
        } catch {
            send(status: "404 Not Found", body: Data(), contentType: "text/plain", connection: connection)
        }
    }

    private func fileURL(for path: String) -> URL? {
        let relative = path == "/" || path == "/mobile" || path == "/mobile/" ? "index.html" : String(path.drop(while: { $0 == "/" }))
        let components = relative.split(separator: "/", omittingEmptySubsequences: true)
        guard !components.contains(".."), !components.contains(".") else { return nil }

        let fileURL = components.reduce(root) { $0.appendingPathComponent(String($1)) }.standardizedFileURL
        guard fileURL.path.hasPrefix(root.path + "/") else { return nil }
        return fileURL
    }

    private func send(status: String, body: Data, contentType: String, connection: NWConnection) {
        let csp = contentType.hasPrefix("text/html")
            ? "Content-Security-Policy: default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self' data:; connect-src 'self' https: wss: ws://127.0.0.1:* http://127.0.0.1:*; object-src 'none'; base-uri 'self'; frame-ancestors 'none'; form-action 'self'\r\n"
            : ""
        let header = "HTTP/1.1 \(status)\r\nContent-Type: \(contentType)\r\nContent-Length: \(body.count)\r\nCache-Control: no-store\r\n\(csp)Connection: close\r\n\r\n"
        connection.send(content: Data(header.utf8) + body, completion: .contentProcessed { _ in connection.cancel() })
    }

    private func mimeType(for fileURL: URL) -> String {
        switch fileURL.pathExtension.lowercased() {
        case "html": "text/html; charset=utf-8"
        case "js", "mjs": "text/javascript; charset=utf-8"
        case "css": "text/css; charset=utf-8"
        case "json": "application/json; charset=utf-8"
        case "svg": "image/svg+xml"
        case "png": "image/png"
        case "jpg", "jpeg": "image/jpeg"
        case "woff": "font/woff"
        case "woff2": "font/woff2"
        default: "application/octet-stream"
        }
    }
}
