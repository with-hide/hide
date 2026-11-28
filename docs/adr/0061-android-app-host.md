# 0061. The Android app host

- Status: Accepted
- Date: 2026-11-13
- Code: `mobile/android/`, `mobile/scripts/`

## Decision

### The Android app host

`mobile/android/` is the same host for Android: a Capacitor project beside the iOS one that shares `mobile/capacitor.config.json`, `assemble.mjs` and `retained-bundles.json`, so the web folder it serves is the iOS app's, and owns nothing of the phone's behavior either.
It is `me.grab.hide.android`, for Android 9 (API 28) and later, compiled for and targeting API 36, and its version is `mobile/android/version.json`.
Capacitor serves the folder from `https://localhost`; `appPlatform()` in `web/src/mobile/appHost.ts` reads the Android app from that origin together with Capacitor's own answer that the platform is Android, so `appHost()` holds there as in the iOS app and every difference above follows it.
The page calls the push plugin only where Capacitor reports a `HidePush` plugin (`appPushAvailable()`), so an Android app built before its notifications is asked nothing about them.
Its native code is `mobile/android/app/src/main/java/me/grab/hide/android/`, whose types carry the iOS names where they do the same job.

The bridge: Capacitor answers the page through a web message listener named `androidBridge`, falls back to a Java object in every frame when the web view lacks that listener, and puts two Java objects of its own plugins (cookies and HTTP) into every frame; it loads any plugin a call names.
The app's web view (`GatedWebView`, put in place of Capacitor's by an app layout of the same name as Capacitor's) refuses every Java object, so none is ever in a page.
`BridgeGate` removes Capacitor's listener and registers itself under the same name for the app's own origin only, and passes a message on only when it comes from the main frame, is a plugin call or the page's error report, and names a plugin in `BridgeGate.APP_PLUGINS` (`HideKeychain`, `HideScanner`, `HideApp`, `HidePush`; there is no `Console`, since Capacitor's Android bridge sends the console to the system log without a plugin).
`mobile/scripts/check-versions.mjs` fails when that list and the app's `@CapacitorPlugin` classes differ, when `capacitor.settings.gradle` names a project beside Capacitor's own library (an npm plugin `cap sync` found), when the app applies `capacitor.build.gradle` (through which `cap sync` links a plugin's library) and when the app links a library outside its fixed list.
A web view without the web message listener gets no bridge at all, and neither does one where Capacitor could not add its listener (it then falls back to its Java object, which the web view refused): the app drops the bridge and shows its own page asking for an updated Android System WebView, with a link to it on Google Play.
The gate is in place before the page is loaded again, because a page keeps the listener it was loaded with.
`BridgeGateWebViewTest`, an instrumented test in a real web view, proves that a same-origin frame and a sandboxed frame reach neither a plugin nor the cookie object while the main frame's call passes, and fails when the gate's install goes; and that a listener Capacitor could not add leaves no bridge.

The navigation: `ShellWebViewClient` carries out `NavigationPolicy` itself, the same rule as iOS: the app's origin loads inside, any other `https` address opens in the phone's browser, and every other scheme is cancelled.
Capacitor's own client would hand `tel:`, `intent:` or another app's scheme to the system, and, comparing no port, load `https://localhost:8443/` inside; `ShellNavigationWebViewTest` drives the real client with that address.
The web view makes no second window, so `window.open` and a link with a target reach the same rule as a navigation.
`PhoneRouter` answers `/p/<n>/` with that bundle's `index.html`, where Capacitor's local server would answer the root page.

The pairing credential is kept by `HideKeychain` (`PairingStore`): Android has no keychain for a small secret, so each computer's item is a file named by its room in the app's no-backup folder, sealed with AES-GCM under one Android Keystore key that never leaves the Keystore and works only while the phone is unlocked, with the room as the cipher's associated data, so a file moved to another room's name does not open.
The calls and their checks are iOS's (`save`, `read`, `remove`, `list`, at most 16 items), and an item is written beside its file and moved over it.
A folder the app cannot read fails a `list` (`keychain_failed`) as the iOS Keychain's does; only a folder not made yet is empty, since an empty answer would have the page drop every saved computer and then its credential.
The folder and the key go with the app when it is deleted, so nothing of an earlier install is left to clear; the app also turns backup off (`allowBackup` false and data extraction rules that exclude every domain from cloud backup and device transfer).
`PairingStoreKeystoreTest` runs on a device against the real Keystore.

`HideScanner` uses Google's code scanner (`play-services-code-scanner`): Google Play services draws the camera view, so the app asks for no camera permission, and the call returns only the text of a code that is a Hide pairing link (`PairLink.accept`), scanning again when it is not.
A pairing link opens the app as an App Link (`https://app.withhide.dev`, path `/`, `autoVerify`; the `assetlinks.json` the hosted site will serve is not part of this app); `ShellActivity` hands it to `PairLinkInbox` and takes it off the intent before Capacitor sees it, on a launch and on a new intent alike.
The inbox is iOS's: the exact link form, at most 1024 bytes, kept one minute and given out once, to the page's `takeLink`.

The notifications are iOS's, through Google's Firebase Cloud Messaging (FCM) in place of Apple's service.
`HidePush` has iOS's calls and answers, and no call that reads a private key, sends an action or takes a key of the caller's own: it reports whether notifications are allowed (`unknown` while Android 13 or later may still ask, `denied` when it will not, or when the app's notifications are off in the system's settings), turns them on for one computer (asks for `POST_NOTIFICATIONS` only then, gets Firebase's token, makes that computer's key and answers `{found, service: "fcm", token, p256dh, auth}`, with no environment since FCM has one host), answers the same registration again on every connection, forgets a computer's key, hands the page the agent a tapped notification named (once), posts the demo's sample, and opens the app's page of the system's notification settings.
Firebase makes no token before the first `enable` (`firebase_messaging_auto_init_enabled` is false until then), keeps it current from then on, and the `forget` that removes the last key deletes the token, so Google answers `UNREGISTERED` to every computer still sending, as the iOS app unregisters from Apple; a changed token reaches each computer in the registration the page sends on its next connection.
`forget` answers once that delete has ended, and an `enable` asked while it runs waits behind it on the plugin's one work thread, so a switch turned off and on again at once gets the next token, never the one being deleted.
The push keys (`PushKeyStore`) are a P-256 key pair and a 16-byte authentication secret per computer, made on the phone, sealed like the pairing credential (`SealedFiles`) but in a folder and under an Android Keystore key of their own, usable while the phone is locked after its first unlock since a notice arrives on a locked phone, at most 16; only `PushKeyStore` reads them, and the message service never touches a credential.
Android has no separate process for this, unlike the iOS extension: the separation is the folder and the key, not the operating system.
`HidePushService`, the app's `FirebaseMessagingService`, receives the relay's data message (`{"hide": <payload>}`, RELAY_PROTOCOL.md), tries each computer's key with `PushCipher` (RFC 8291, the twin of `NotificationCipher`, which also refuses a sender's point off the curve), and shows the notice only when the key that opened it is the room the notice names (`NoticeOpener`), worded from `NoticeWords`, the iOS table in the language of the phone's system, which `NoticeWordsTest` holds equal to `NoticeWords.swift` key for key.
A notice replaces the notification of the same tag and closes the tags in its `clear` list, as the web app's service worker does; a body no key opens shows the fixed text (`notice_fixed`, iOS's `hide-notification`) under one tag, never nothing, since FCM lowers the priority of an app whose high-priority messages show nothing.
Android drops an update of a shown notification that comes faster than five a second (`Shedding` in the system log), so when FCM hands over a burst of held messages at once, as to a phone back online, an agent's newer notice can be dropped and its earlier one stay; its tap still opens that agent, and a notice under a new tag is a new notification, which this limit does not drop.
Nothing decrypted is logged: a log line names what happened, never a title, an agent or a room.
A tap goes to `NoticeOpenActivity`, which is not exported, so only the app's own notifications name an agent; it hands the agent to `PushOpenInbox`, which the page asks once (`takeOpen`) after the `open` event, and brings `ShellActivity` forward.
The Firebase values (app id, project, sender id and API key) are `res/values/firebase.xml`, what Google's Gradle plugin would write from `google-services.json`; they ship inside every APK and are public by design, and the key is restricted in Google Cloud to `me.grab.hide.android` signed by the listed certificates and to the Firebase Installations and FCM registration APIs ([BUILD.md](../BUILD.md#the-android-apps-notifications)).
The app links Firebase Messaging and no analytics or advertising library, and the manifest still removes the advertising id permission; Firebase adds `ACCESS_NETWORK_STATE`, `WAKE_LOCK` and `com.google.android.c2dm.permission.RECEIVE`.
A phone without Google Play services gets no token: `enable` answers `token_unavailable` and the switch reads failed.
`HidePushPluginTest`, an instrumented test, turns notifications on, then off and on again without waiting, through the page's bridge and Firebase's real token calls, and checks the second `enable` got the token Firebase holds afterwards.
`HidePushServiceTest`, an instrumented test, hands the service messages sealed as hided seals them and reads the system's notifications back, and sends a notification's own tap, which reaches a page through `takeOpen`; the delivery through FCM is proved on the build machine's emulator (Google Play image).

`HideApp.appearance` sets the system bars' icons to the page's light or dark and the color behind them; Capacitor's `SystemBars` leaves room for the bars and the keyboard, by letting the page draw under the bars on a web view of version 140 or later, which reports `env(safe-area-inset-*)`, and by padding the window on an older one, where the native color behind the bars is the page's background in its default color theme.
The system back presses the page's visible `data-phone-back` button, within the top open sheet, closes a sheet with no back button as Escape does, and with nothing to go back to moves the app to the background; the page keeps owning its back action, as on iOS.
A phone (smallest width under 600 dp) stays upright.
The release build has no cleartext traffic, no web view debugging, no Capacitor logging and no advertising id permission; a debug build allows cleartext to `127.0.0.1` for a relay fixture and has the debug inputs (`src/debug`, `DebugInputs`) the build script's `run` uses, which a release build does not contain.
The build is in [BUILD.md](../BUILD.md#the-android-app).
