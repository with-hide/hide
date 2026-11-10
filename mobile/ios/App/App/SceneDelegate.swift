import UIKit
import Capacitor
import HideShell

class SceneDelegate: UIResponder, UIWindowSceneDelegate {
    var window: UIWindow?

    func scene(_ scene: UIScene, willConnectTo session: UISceneSession, options connectionOptions: UIScene.ConnectionOptions) {
        guard let windowScene = scene as? UIWindowScene else { return }

        window = UIWindow(windowScene: windowScene)
        window?.rootViewController = ShellViewController()
        window?.makeKeyAndVisible()

        SceneDelegateProxy.shared.scene(scene, willConnectTo: session, options: connectionOptions)
        // The app was started by a link: the web app asks for it once the page is up (`HideApp.takeLink`).
        for activity in connectionOptions.userActivities where activity.activityType == NSUserActivityTypeBrowsingWeb {
            PairLinkInbox.shared.offer(activity.webpageURL)
        }
    }

    func scene(_ scene: UIScene, openURLContexts URLContexts: Set<UIOpenURLContext>) {
        SceneDelegateProxy.shared.scene(scene, openURLContexts: URLContexts)
    }

    func scene(_ scene: UIScene, continue userActivity: NSUserActivity) {
        // A universal link, the only way a link opens the app (the app registers no URL scheme of its own). Only a
        // pairing link of the hosted phone site is kept; the web app decodes it and a person presses Connect.
        if userActivity.activityType == NSUserActivityTypeBrowsingWeb { PairLinkInbox.shared.offer(userActivity.webpageURL) }
        SceneDelegateProxy.shared.scene(scene, continue: userActivity)
    }
}
