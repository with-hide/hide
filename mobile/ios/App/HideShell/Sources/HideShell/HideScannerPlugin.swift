import AVFoundation
import Capacitor
import UIKit

/// The words the scanner draws, in the language the web app is showing: the page sends them with the call, so the
/// native side owns no catalog of its own and cannot disagree with the page about a word.
struct ScannerText {
    static let keys = ["title", "hint", "whereTitle", "whereBody", "deniedTitle", "deniedBody", "openSettings", "close"]
    let title, hint, whereTitle, whereBody, deniedTitle, deniedBody, openSettings, close: String

    /// Nil unless every word is there, each cut to a length a label can hold.
    init?(_ values: [String: Any]?) {
        guard let values else { return nil }
        var found: [String: String] = [:]
        for key in Self.keys {
            guard let value = values[key] as? String, !value.isEmpty else { return nil }
            found[key] = String(value.prefix(160))
        }
        title = found["title"]!; hint = found["hint"]!; whereTitle = found["whereTitle"]!; whereBody = found["whereBody"]!
        deniedTitle = found["deniedTitle"]!; deniedBody = found["deniedBody"]!; openSettings = found["openSettings"]!; close = found["close"]!
    }
}

/// The camera scanner behind the QR scan button: one full-screen view that reads one Hide pairing link and gives its
/// text back, which the web app decodes with the decoder every pairing uses. The camera permission is asked here,
/// when a person has pressed the button, and never at launch. A refused or unusable camera is a screen that says
/// what to do (and opens the app's page in iPhone Settings), never a black view: every way out of the view, the
/// close button included, ends the promise and frees the next press.
@objc(HideScannerPlugin)
open class HideScannerPlugin: CAPPlugin, CAPBridgedPlugin {
    public let identifier = "HideScannerPlugin"
    public let jsName = "HideScanner"
    public let pluginMethods: [CAPPluginMethod] = [
        CAPPluginMethod(name: "scan", returnType: CAPPluginReturnPromise),
    ]

    private var scanning = false

    @objc func scan(_ call: CAPPluginCall) {
        guard let text = ScannerText(call.getObject("text")) else { call.reject("invalid_text", "invalid_text"); return }
        DispatchQueue.main.async { [weak self] in
            guard let self, !self.scanning else { call.reject("busy", "busy"); return }
            guard let presenter = self.bridge?.viewController, presenter.presentedViewController == nil else { call.reject("busy", "busy"); return }
            self.scanning = true
            self.authorize { allowed in
                let scanner = QRScannerViewController(text: text, authorized: allowed, allowsNoCamera: Self.simulatorStandIn) { [weak self] value in
                    self?.scanning = false
                    if let value { call.resolve(["value": value]) } else { call.reject("cancelled", "cancelled") }
                }
                scanner.modalPresentationStyle = .fullScreen
                presenter.present(scanner, animated: true)
            }
        }
    }

    /// A simulator has no camera: the debug build still shows the scanner's own screen, for the capture.
    private static var simulatorStandIn: Bool {
#if DEBUG && targetEnvironment(simulator)
        return true
#else
        return false
#endif
    }

    private func authorize(_ done: @escaping (Bool) -> Void) {
        switch AVCaptureDevice.authorizationStatus(for: .video) {
        case .authorized: done(true)
        case .notDetermined: AVCaptureDevice.requestAccess(for: .video) { granted in DispatchQueue.main.async { done(granted) } }
        default: done(false)
        }
    }
}

/// Reads QR codes until one is a Hide pairing link, then closes itself; a code that is anything else is ignored and
/// the camera keeps looking. Without a camera it shows the screen that says why, with the way to iPhone Settings.
final class QRScannerViewController: UIViewController, AVCaptureMetadataOutputObjectsDelegate {
    enum Mode { case scanning, unavailable }

    private let text: ScannerText
    private let makeInput: () -> AVCaptureDeviceInput?
    private let openSettings: () -> Void
    private let allowsNoCamera: Bool
    private var completion: ((String?) -> Void)?
    private(set) var mode: Mode
    private let session = AVCaptureSession()
    private let queue = DispatchQueue(label: "me.grab.hide.ios.scanner")
    private var preview: AVCaptureVideoPreviewLayer?
    private var output: AVCaptureMetadataOutput?

    private let dim = CAShapeLayer()
    private let brackets = CAShapeLayer()
    private let aim = UIView()
    private let scanViews = UIView()
    private let deniedViews = UIView()
    private(set) var closeButton = UIButton(type: .system)
    private(set) var settingsButton = UIButton(type: .system)

    /// `authorized`: the camera permission is granted. `allowsNoCamera`: the debug simulator, which has no camera and
    /// still shows the scanning screen. `makeInput` and `openSettings` are the system's in the app and stand-ins in tests.
    init(text: ScannerText, authorized: Bool, allowsNoCamera: Bool = false,
         makeInput: @escaping () -> AVCaptureDeviceInput? = { AVCaptureDevice.default(for: .video).flatMap { try? AVCaptureDeviceInput(device: $0) } },
         openSettings: @escaping () -> Void = { if let url = URL(string: UIApplication.openSettingsURLString) { UIApplication.shared.open(url) } },
         completion: @escaping (String?) -> Void) {
        self.text = text
        self.allowsNoCamera = allowsNoCamera
        self.makeInput = makeInput
        self.openSettings = openSettings
        self.completion = completion
        self.mode = authorized ? .scanning : .unavailable
        super.init(nibName: nil, bundle: nil)
    }

    required init?(coder: NSCoder) { fatalError("not from a storyboard") }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = UIColor(red: 0.07, green: 0.075, blue: 0.07, alpha: 1)
        if mode == .scanning { startCamera() }
        buildChrome()
        render()
    }

    /// The camera is set up before any view is on screen, and a failure only changes the screen: nothing here ends the scan.
    private func startCamera() {
        guard let input = makeInput(), session.canAddInput(input) else { if !allowsNoCamera { mode = .unavailable }; return }
        session.addInput(input)
        let output = AVCaptureMetadataOutput()
        guard session.canAddOutput(output) else { mode = .unavailable; return }
        session.addOutput(output)
        output.setMetadataObjectsDelegate(self, queue: .main)
        output.metadataObjectTypes = [.qr]
        self.output = output
        let layer = AVCaptureVideoPreviewLayer(session: session)
        layer.videoGravity = .resizeAspectFill
        view.layer.addSublayer(layer)
        preview = layer
    }

    private func label(_ string: String, style: UIFont.TextStyle, weight: UIFont.Weight, alpha: CGFloat = 1) -> UILabel {
        let label = UILabel()
        label.text = string
        label.textColor = UIColor.white.withAlphaComponent(alpha)
        label.numberOfLines = 0
        label.textAlignment = .center
        label.adjustsFontForContentSizeCategory = true
        label.font = UIFontMetrics(forTextStyle: style).scaledFont(for: .systemFont(ofSize: UIFont.preferredFont(forTextStyle: style).pointSize, weight: weight))
        label.translatesAutoresizingMaskIntoConstraints = false
        return label
    }

    private func buildChrome() {
        dim.fillRule = .evenOdd
        dim.fillColor = UIColor.black.withAlphaComponent(0.42).cgColor
        brackets.strokeColor = UIColor.white.cgColor
        brackets.fillColor = nil
        brackets.lineWidth = 3
        brackets.lineCap = .round
        view.layer.addSublayer(dim)
        view.layer.addSublayer(brackets)
        aim.isUserInteractionEnabled = false
        view.addSubview(aim)

        for container in [scanViews, deniedViews] {
            container.translatesAutoresizingMaskIntoConstraints = false
            container.isUserInteractionEnabled = container === deniedViews
            view.addSubview(container)
            NSLayoutConstraint.activate([
                container.leadingAnchor.constraint(equalTo: view.leadingAnchor), container.trailingAnchor.constraint(equalTo: view.trailingAnchor),
                container.topAnchor.constraint(equalTo: view.topAnchor), container.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            ])
        }

        // Scanning: a title and a line under it, and where the QR is at the bottom, around the aim frame.
        let title = label(text.title, style: .headline, weight: .semibold)
        title.accessibilityIdentifier = "scanner.title"
        let hint = label(text.hint, style: .footnote, weight: .regular, alpha: 0.72)
        let box = UIView()
        box.backgroundColor = UIColor.white.withAlphaComponent(0.14)
        box.layer.cornerRadius = 14
        box.translatesAutoresizingMaskIntoConstraints = false
        let whereTitle = label(text.whereTitle, style: .subheadline, weight: .semibold)
        let whereBody = label(text.whereBody, style: .footnote, weight: .regular)
        for item in [title, hint, box] { scanViews.addSubview(item) }
        for item in [whereTitle, whereBody] { box.addSubview(item) }
        let guide = view.safeAreaLayoutGuide
        NSLayoutConstraint.activate([
            title.topAnchor.constraint(equalTo: guide.topAnchor, constant: 64),
            title.leadingAnchor.constraint(equalTo: guide.leadingAnchor, constant: 24), title.trailingAnchor.constraint(equalTo: guide.trailingAnchor, constant: -24),
            hint.topAnchor.constraint(equalTo: title.bottomAnchor, constant: 4),
            hint.leadingAnchor.constraint(equalTo: title.leadingAnchor), hint.trailingAnchor.constraint(equalTo: title.trailingAnchor),
            box.leadingAnchor.constraint(equalTo: guide.leadingAnchor, constant: 24), box.trailingAnchor.constraint(equalTo: guide.trailingAnchor, constant: -24),
            box.bottomAnchor.constraint(equalTo: guide.bottomAnchor, constant: -24),
            whereTitle.topAnchor.constraint(equalTo: box.topAnchor, constant: 12),
            whereTitle.leadingAnchor.constraint(equalTo: box.leadingAnchor, constant: 14), whereTitle.trailingAnchor.constraint(equalTo: box.trailingAnchor, constant: -14),
            whereBody.topAnchor.constraint(equalTo: whereTitle.bottomAnchor, constant: 2),
            whereBody.leadingAnchor.constraint(equalTo: whereTitle.leadingAnchor), whereBody.trailingAnchor.constraint(equalTo: whereTitle.trailingAnchor),
            whereBody.bottomAnchor.constraint(equalTo: box.bottomAnchor, constant: -12),
        ])

        // Unavailable: what to do, and the way to iPhone Settings, in the middle of a plain screen.
        let deniedTitle = label(text.deniedTitle, style: .headline, weight: .semibold)
        let deniedBody = label(text.deniedBody, style: .subheadline, weight: .regular, alpha: 0.72)
        settingsButton.configuration = { var c = UIButton.Configuration.filled(); c.title = text.openSettings; c.baseBackgroundColor = .white; c.baseForegroundColor = UIColor(red: 0.07, green: 0.075, blue: 0.07, alpha: 1); c.cornerStyle = .large; return c }()
        settingsButton.addAction(UIAction { [weak self] _ in self?.openSettings() }, for: .touchUpInside)
        settingsButton.accessibilityIdentifier = "scanner.settings"
        settingsButton.translatesAutoresizingMaskIntoConstraints = false
        for item in [deniedTitle, deniedBody, settingsButton] { deniedViews.addSubview(item) }
        NSLayoutConstraint.activate([
            deniedTitle.centerYAnchor.constraint(equalTo: deniedViews.centerYAnchor, constant: -40),
            deniedTitle.leadingAnchor.constraint(equalTo: guide.leadingAnchor, constant: 24), deniedTitle.trailingAnchor.constraint(equalTo: guide.trailingAnchor, constant: -24),
            deniedBody.topAnchor.constraint(equalTo: deniedTitle.bottomAnchor, constant: 6),
            deniedBody.leadingAnchor.constraint(equalTo: deniedTitle.leadingAnchor), deniedBody.trailingAnchor.constraint(equalTo: deniedTitle.trailingAnchor),
            settingsButton.topAnchor.constraint(equalTo: deniedBody.bottomAnchor, constant: 22),
            settingsButton.leadingAnchor.constraint(equalTo: guide.leadingAnchor, constant: 48), settingsButton.trailingAnchor.constraint(equalTo: guide.trailingAnchor, constant: -48),
            settingsButton.heightAnchor.constraint(greaterThanOrEqualToConstant: 48),
        ])

        // The close button is built in every mode, so no state of the camera leaves the person with no way out.
        closeButton.configuration = { var c = UIButton.Configuration.gray(); c.image = UIImage(systemName: "xmark"); c.cornerStyle = .capsule; c.baseForegroundColor = .white; return c }()
        closeButton.addAction(UIAction { [weak self] _ in self?.finish(nil) }, for: .touchUpInside)
        closeButton.accessibilityLabel = text.close
        closeButton.accessibilityIdentifier = "scanner.close"
        closeButton.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(closeButton)
        NSLayoutConstraint.activate([
            closeButton.topAnchor.constraint(equalTo: guide.topAnchor, constant: 12),
            closeButton.trailingAnchor.constraint(equalTo: guide.trailingAnchor, constant: -16),
            closeButton.widthAnchor.constraint(equalToConstant: 44), closeButton.heightAnchor.constraint(equalToConstant: 44),
        ])
    }

    /// Shows the screen for the mode: the aim frame and the dimmed rest while scanning, the plain guidance otherwise.
    private func render() {
        let scanning = mode == .scanning
        scanViews.isHidden = !scanning
        deniedViews.isHidden = scanning
        dim.isHidden = !scanning
        brackets.isHidden = !scanning
        preview?.isHidden = !scanning
        view.bringSubviewToFront(closeButton)
        view.setNeedsLayout()
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        preview?.frame = view.bounds
        let side = min(view.bounds.width - 96, 240)
        let frame = CGRect(x: (view.bounds.width - side) / 2, y: view.bounds.height * 0.44 - side / 2, width: side, height: side)
        aim.frame = frame
        let hole = UIBezierPath(roundedRect: frame, cornerRadius: 22)
        let path = UIBezierPath(rect: view.bounds)
        path.append(hole)
        dim.frame = view.bounds
        dim.path = path.cgPath
        let corner: CGFloat = 34, r: CGFloat = 22
        let b = UIBezierPath()
        for (x, y, dx, dy) in [(frame.minX, frame.minY, 1.0, 1.0), (frame.maxX, frame.minY, -1.0, 1.0), (frame.minX, frame.maxY, 1.0, -1.0), (frame.maxX, frame.maxY, -1.0, -1.0)] {
            b.move(to: CGPoint(x: x + dx * corner, y: y))
            b.addLine(to: CGPoint(x: x + dx * r, y: y))
            b.addQuadCurve(to: CGPoint(x: x, y: y + dy * r), controlPoint: CGPoint(x: x, y: y))
            b.addLine(to: CGPoint(x: x, y: y + dy * corner))
        }
        brackets.path = b.cgPath
        // Only what is inside the frame is read.
        if let preview, let output { output.rectOfInterest = preview.metadataOutputRectConverted(fromLayerRect: frame) }
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        if mode == .scanning, preview != nil {
            let session = session
            queue.async { if !session.isRunning { session.startRunning() } }
        }
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        let session = session
        queue.async { if session.isRunning { session.stopRunning() } }
    }

    /// Any way the view leaves the screen without the close button or a code, so the page's promise still ends.
    override func viewDidDisappear(_ animated: Bool) {
        super.viewDidDisappear(animated)
        if isBeingDismissed || isMovingFromParent { finish(nil) }
    }

    override var supportedInterfaceOrientations: UIInterfaceOrientationMask { .portrait }
    override var prefersStatusBarHidden: Bool { true }

    func metadataOutput(_ output: AVCaptureMetadataOutput, didOutput objects: [AVMetadataObject], from connection: AVCaptureConnection) {
        for object in objects {
            if let code = object as? AVMetadataMachineReadableCodeObject, let value = code.stringValue, handle(value) { return }
        }
    }

    /// A code that is a Hide pairing link ends the scan; any other code is ignored and the camera keeps looking.
    @discardableResult
    func handle(_ value: String) -> Bool {
        guard let link = PairLink.accept(URL(string: value)) else { return false }
        finish(link)
        return true
    }

    /// Ends the scan once: the answer reaches the plugin whichever way the view closed.
    func finish(_ value: String?) {
        guard let done = completion else { return }
        completion = nil
        if presentingViewController != nil { dismiss(animated: true) { done(value) } } else { done(value) }
    }
}
