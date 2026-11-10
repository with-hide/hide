import AVFoundation
import Capacitor
import UIKit

/// The camera scanner behind the 스캔 button: one full-screen camera view that reads one QR code and gives its text
/// back, which the web app decodes with the decoder every pairing uses. The camera permission is asked here, when a
/// person has pressed the button, and never at launch.
@objc(HideScannerPlugin)
open class HideScannerPlugin: CAPPlugin, CAPBridgedPlugin {
    public let identifier = "HideScannerPlugin"
    public let jsName = "HideScanner"
    public let pluginMethods: [CAPPluginMethod] = [
        CAPPluginMethod(name: "scan", returnType: CAPPluginReturnPromise),
    ]

    private var scanning = false

    @objc func scan(_ call: CAPPluginCall) {
        let hint = String((call.getString("hint") ?? "").prefix(120))
        DispatchQueue.main.async { [weak self] in
            guard let self, !self.scanning else { call.reject("busy", "busy"); return }
            self.scanning = true
            self.authorize { allowed in
                guard allowed else { self.finish(call, code: "camera_denied"); return }
                guard AVCaptureDevice.default(for: .video) != nil else { self.finish(call, code: "camera_unavailable"); return }
                guard let presenter = self.bridge?.viewController else { self.finish(call, code: "camera_unavailable"); return }
                let scanner = QRScannerViewController(hint: hint) { [weak self] value in
                    guard let self else { return }
                    self.scanning = false
                    if let value { call.resolve(["value": value]) } else { call.reject("cancelled", "cancelled") }
                }
                scanner.modalPresentationStyle = .fullScreen
                presenter.present(scanner, animated: true)
            }
        }
    }

    private func finish(_ call: CAPPluginCall, code: String) {
        scanning = false
        call.reject(code, code)
    }

    private func authorize(_ done: @escaping (Bool) -> Void) {
        switch AVCaptureDevice.authorizationStatus(for: .video) {
        case .authorized: done(true)
        case .notDetermined: AVCaptureDevice.requestAccess(for: .video) { granted in DispatchQueue.main.async { done(granted) } }
        default: done(false)
        }
    }
}

/// Reads QR codes until one has text of a size a pairing link can have, then closes itself.
final class QRScannerViewController: UIViewController, AVCaptureMetadataOutputObjectsDelegate {
    private let hint: String
    private var completion: ((String?) -> Void)?
    private let session = AVCaptureSession()
    private let queue = DispatchQueue(label: "me.grab.hide.ios.scanner")
    private var preview: AVCaptureVideoPreviewLayer?

    init(hint: String, completion: @escaping (String?) -> Void) {
        self.hint = hint
        self.completion = completion
        super.init(nibName: nil, bundle: nil)
    }

    required init?(coder: NSCoder) { fatalError("not from a storyboard") }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .black
        guard let device = AVCaptureDevice.default(for: .video), let input = try? AVCaptureDeviceInput(device: device),
              session.canAddInput(input) else { return finish(nil) }
        session.addInput(input)
        let output = AVCaptureMetadataOutput()
        guard session.canAddOutput(output) else { return finish(nil) }
        session.addOutput(output)
        output.setMetadataObjectsDelegate(self, queue: .main)
        output.metadataObjectTypes = [.qr]
        let layer = AVCaptureVideoPreviewLayer(session: session)
        layer.videoGravity = .resizeAspectFill
        view.layer.addSublayer(layer)
        preview = layer

        let label = UILabel()
        label.text = hint
        label.textColor = .white
        label.numberOfLines = 0
        label.textAlignment = .center
        label.font = .preferredFont(forTextStyle: .body)
        label.translatesAutoresizingMaskIntoConstraints = false
        let cancel = UIButton(type: .system, primaryAction: UIAction(title: "") { [weak self] _ in self?.finish(nil) })
        cancel.configuration = .gray()
        cancel.configuration?.image = UIImage(systemName: "xmark")
        cancel.accessibilityLabel = NSLocalizedString("Close", tableName: nil, bundle: Bundle(for: UIView.self), value: "Close", comment: "")
        cancel.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(label)
        view.addSubview(cancel)
        NSLayoutConstraint.activate([
            cancel.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 12),
            cancel.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -16),
            cancel.widthAnchor.constraint(equalToConstant: 44),
            cancel.heightAnchor.constraint(equalToConstant: 44),
            label.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 24),
            label.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -24),
            label.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor, constant: -32),
        ])
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        preview?.frame = view.bounds
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        let session = session
        queue.async { if !session.isRunning { session.startRunning() } }
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        let session = session
        queue.async { if session.isRunning { session.stopRunning() } }
    }

    override var supportedInterfaceOrientations: UIInterfaceOrientationMask { .portrait }
    override var prefersStatusBarHidden: Bool { true }

    func metadataOutput(_ output: AVCaptureMetadataOutput, didOutput objects: [AVMetadataObject], from connection: AVCaptureConnection) {
        for object in objects {
            guard let code = object as? AVMetadataMachineReadableCodeObject, let text = code.stringValue,
                  text.utf8.count <= PairLink.maxBytes else { continue }
            return finish(text)
        }
    }

    private func finish(_ value: String?) {
        guard let done = completion else { return }
        completion = nil
        dismiss(animated: true) { done(value) }
    }
}
