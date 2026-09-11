import AVFoundation
import SwiftUI
import UIKit

struct QRScannerView: UIViewControllerRepresentable {
    let onScan: (String) -> Void
    let onCancel: () -> Void

    func makeUIViewController(context: Context) -> QRScannerViewController {
        QRScannerViewController(onScan: onScan, onCancel: onCancel)
    }

    func updateUIViewController(_ uiViewController: QRScannerViewController, context: Context) {}
}

final class QRScannerViewController: UIViewController, AVCaptureMetadataOutputObjectsDelegate {
    private let onScan: (String) -> Void
    private let onCancel: () -> Void
    private let captureSession = AVCaptureSession()
    private let sessionQueue = DispatchQueue(label: "com.soyrex.monitter.qr-camera")
    private let statusLabel = UILabel()
    private let cancelButton = UIButton(type: .system)
    private var previewLayer: AVCaptureVideoPreviewLayer?
    private var configured = false
    private var wantsRunning = false
    private var completed = false

    init(onScan: @escaping (String) -> Void, onCancel: @escaping () -> Void) {
        self.onScan = onScan
        self.onCancel = onCancel
        super.init(nibName: nil, bundle: nil)
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .black
        installControls()
        requestCameraAccess()
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        sessionQueue.async { [weak self] in
            guard let self else { return }
            self.wantsRunning = true
            self.startIfNeeded()
        }
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        sessionQueue.async { [weak self] in
            guard let self else { return }
            self.wantsRunning = false
            self.stopIfNeeded()
        }
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        previewLayer?.frame = view.bounds
    }

    private func installControls() {
        statusLabel.text = "Preparing camera…"
        statusLabel.textColor = .white
        statusLabel.font = .preferredFont(forTextStyle: .body)
        statusLabel.numberOfLines = 0
        statusLabel.textAlignment = .center
        statusLabel.translatesAutoresizingMaskIntoConstraints = false

        var buttonConfiguration = UIButton.Configuration.filled()
        buttonConfiguration.title = "Cancel"
        buttonConfiguration.baseForegroundColor = .white
        buttonConfiguration.baseBackgroundColor = UIColor.black.withAlphaComponent(0.55)
        buttonConfiguration.cornerStyle = .medium
        buttonConfiguration.contentInsets = NSDirectionalEdgeInsets(top: 12, leading: 24, bottom: 12, trailing: 24)
        cancelButton.configuration = buttonConfiguration
        cancelButton.translatesAutoresizingMaskIntoConstraints = false
        cancelButton.addTarget(self, action: #selector(cancelTapped), for: .touchUpInside)

        view.addSubview(statusLabel)
        view.addSubview(cancelButton)
        NSLayoutConstraint.activate([
            cancelButton.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            cancelButton.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor, constant: -24),
            statusLabel.leadingAnchor.constraint(equalTo: view.layoutMarginsGuide.leadingAnchor),
            statusLabel.trailingAnchor.constraint(equalTo: view.layoutMarginsGuide.trailingAnchor),
            statusLabel.bottomAnchor.constraint(equalTo: cancelButton.topAnchor, constant: -20),
        ])
    }

    private func requestCameraAccess() {
        switch AVCaptureDevice.authorizationStatus(for: .video) {
        case .authorized:
            configureOnSessionQueue()
        case .notDetermined:
            showStatus("Allow camera access to scan a Monitter invitation.")
            AVCaptureDevice.requestAccess(for: .video) { [weak self] granted in
                guard let self else { return }
                if granted { self.configureOnSessionQueue() }
                else { self.showFallback("Camera access was not granted. Tap Cancel and paste the pairing code instead.") }
            }
        case .denied, .restricted:
            showFallback("Camera access is unavailable. Tap Cancel and paste the pairing code instead.")
        @unknown default:
            showFallback("Camera access is unavailable. Tap Cancel and paste the pairing code instead.")
        }
    }

    private func configureOnSessionQueue() {
        sessionQueue.async { [weak self] in
            guard let self, !self.completed else { return }
            self.configureCamera()
            self.startIfNeeded()
        }
    }

    private func configureCamera() {
        guard !configured else { return }
        captureSession.beginConfiguration()
        defer { captureSession.commitConfiguration() }
        guard let camera = AVCaptureDevice.default(for: .video),
              let input = try? AVCaptureDeviceInput(device: camera),
              captureSession.canAddInput(input) else {
            showFallback("No camera is available. Tap Cancel and paste the pairing code instead.")
            return
        }
        captureSession.addInput(input)
        let output = AVCaptureMetadataOutput()
        guard captureSession.canAddOutput(output) else {
            showFallback("The camera could not start. Tap Cancel and paste the pairing code instead.")
            return
        }
        captureSession.addOutput(output)
        output.setMetadataObjectsDelegate(self, queue: .main)
        output.metadataObjectTypes = [.qr]
        configured = true

        DispatchQueue.main.async { [weak self] in
            guard let self, self.previewLayer == nil else { return }
            let preview = AVCaptureVideoPreviewLayer(session: self.captureSession)
            preview.videoGravity = .resizeAspectFill
            preview.frame = self.view.bounds
            self.view.layer.insertSublayer(preview, at: 0)
            self.previewLayer = preview
            self.showStatus("Point your camera at the Monitter invitation QR code.")
        }
    }

    private func startIfNeeded() {
        guard configured, wantsRunning, !completed, !captureSession.isRunning else { return }
        captureSession.startRunning()
    }

    private func stopIfNeeded() {
        guard captureSession.isRunning else { return }
        captureSession.stopRunning()
    }

    func metadataOutput(_ output: AVCaptureMetadataOutput, didOutput metadataObjects: [AVMetadataObject], from connection: AVCaptureConnection) {
        guard !completed,
              let code = metadataObjects.compactMap({ $0 as? AVMetadataMachineReadableCodeObject }).first?.stringValue else { return }
        completed = true
        sessionQueue.async { [weak self] in self?.stopIfNeeded() }
        onScan(code)
    }

    @objc private func cancelTapped() {
        guard !completed else { return }
        completed = true
        sessionQueue.async { [weak self] in self?.stopIfNeeded() }
        onCancel()
    }

    private func showStatus(_ message: String) {
        DispatchQueue.main.async { [weak self] in self?.statusLabel.text = message }
    }

    private func showFallback(_ message: String) {
        DispatchQueue.main.async { [weak self] in self?.statusLabel.text = message }
    }
}
