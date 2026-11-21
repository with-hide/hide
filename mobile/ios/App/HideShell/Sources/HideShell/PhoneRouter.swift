import Capacitor
import Foundation

/// Which file the app's own scheme serves for an address.
///
/// Capacitor's router is made for single-page apps: an address with no file extension answers the root
/// `index.html`. The phone site is not one page. Its loader is the root page and every protocol bundle is a page
/// of its own at `/p/<n>/`, so the loader's `location.replace("/p/5/?host=...")` would load the loader again.
/// `/p/<n>/` answers that bundle's `index.html`; every other address keeps Capacitor's rule.
public struct PhoneRouter: Router {
    public var basePath: String = ""
    private let bundle = try! NSRegularExpression(pattern: "^/p/[1-9][0-9]*/?$")

    public init() {}

    public func route(for path: String) -> String {
        let whole = NSRange(path.startIndex..., in: path)
        if bundle.firstMatch(in: path, range: whole) != nil {
            return basePath + (path.hasSuffix("/") ? path : path + "/") + "index.html"
        }
        return CapacitorRouter(basePath: basePath).route(for: path)
    }
}

extension CapacitorRouter {
    fileprivate init(basePath: String) {
        self.init()
        self.basePath = basePath
    }
}
