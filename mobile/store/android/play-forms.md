# Play Console form answers

The answers for the Play Console forms that are not page text (Policy and programs > App content, and Store settings), with the reason for each.
The page text itself is `metadata/`, the folder layout fastlane's `supply` reads; `mobile/scripts/check-store-text.mjs` holds it to Play's limits.
Nothing here is uploaded by any command; the person who owns the Play Console account enters it there.
Play changes the wording and the order of its questions; the reasons below are the stable part, so read each question for what it asks before choosing the answer.

What the app is, for every answer: the phone side of a program the person runs on their own computer.
It shows the person's own coding agents, their conversations, missions and pull requests, and sends back what the person types.
It has no account, no ads, no purchases, no feed and no way to reach another person.
Its manifest asks for `INTERNET` only (notifications add `POST_NOTIFICATIONS`), removes the advertising ID permission, and allows no backup (`mobile/android/app/src/main/AndroidManifest.xml`).

## Data safety

The form asks what the app collects or shares.
Google's rule: data that leaves the phone only end-to-end encrypted, with no key the developer holds, is not collected.
The conversations, commands, files and pull request text the app shows travel between the phone and the person's own computer through the relay under a Noise session whose keys only those two hold, so none of it is on the form.

| Question | Answer | Reason |
| --- | --- | --- |
| Does the app collect or share any of the required user data types? | Yes | The notification registration below. |
| Is all of the user data collected encrypted in transit? | Yes | The registration travels over TLS (the relay, and Google's push service). |
| Do you provide a way for users to request that their data is deleted? | No, with the reason that follows when the form asks for it | There is no account. Hide's servers keep no registration: the relay passes the token on and stores nothing, and the person's own computer holds it until the person turns Notifications off in the app's Settings or removes the computer. Confirm the wording the Console shows for data that is only processed ephemerally. |
| Data type: Device or other IDs (the push token and the Firebase installation ID) | Collected: yes. Shared: no. Processed ephemerally: yes. Optional: yes, the person turns Notifications on or off. Purpose: App functionality | Approved by the operator 2026-11-13. The token is what lets a computer's agent question reach the phone; the service that carries it is a provider acting for the app, which Google does not count as sharing. |
| Every other data type (location, personal info, financial info, messages, photos, files, contacts, app activity, web history, audio, health, calendar) | Not collected, not shared | The app has no code that reads or sends any of it. The conversations are the first exception above, by the end-to-end rule. |

To decide before the form is saved (not part of the operator's approval above): the QR scanner is Google's code scanner from Play services (`play-services-code-scanner`), and Google's ML Kit disclosure says its SDKs collect device information and error codes for diagnostics, and a scan session ID, which Google asks the app's developer to account for on the form.
The recommendation is to add Diagnostics (crash logs and diagnostics), collected, not shared, purpose Analytics or App functionality as Google's table names it, and to read [the ML Kit disclosure](https://developers.google.com/ml-kit/android-data-disclosure) for the code scanner's current list when the form is filled.
The camera permission is not asked: the scanner runs in Play services.

## Content rating (IARC)

Questionnaire: Policy and programs > App content > Content rating.
The expected result is the lowest rating in every system: IARC 3+ and ESRB Everyone.
The category is Utility, Productivity, Communication or Other, whichever Play lists that is not a game, a social or dating app, or a news app: Utility or Productivity.

| Question | Answer | Reason |
| --- | --- | --- |
| Violence (cartoon, fantasy or realistic, including blood and gore) | No | The app draws no violence. |
| Sexual content or nudity | No | The app draws none. |
| Profanity or crude humor | No | The app's own text has none. An agent's text is the person's own and is not content the app provides. |
| Controlled substances (drugs, alcohol, tobacco) | No | None in the app. |
| Gambling or simulated gambling | No | None. |
| Fear or horror themes | No | None in the app. |
| Does the app let users interact or exchange content with each other? | No | Nothing a person writes reaches another person. The conversations are between the person and their own agent on their own computer. |
| Does the app share the user's location with other users? | No | The app reads no location. |
| Does the app allow users to purchase digital goods? | No | There are no purchases. |
| Does the app provide unrestricted web access? | No | The app has no browser. Its one web view shows the app's own pages and, on the "To look at" tab, a page the person's own agent made on their own computer, in a sandboxed frame. |
| Does the app contain user-generated content moderated or shown to others? | No | Same as the interaction row. |

The text of an agent's conversation is produced by the person's own agent on the person's own computer, and the app shows it as it came; answers that depend on whether the app generates content use that fact.

## Target audience and content

| Question | Answer | Reason |
| --- | --- | --- |
| Target age groups | 18 and over only | A coding tool for working developers. Choosing no group under 18 keeps the app outside the Families policy, which applies once a child is a target. |
| Does the app appeal to children? | No | Nothing in the page or the app is made for children. |

## Other App content declarations

| Form | Answer | Reason |
| --- | --- | --- |
| Ads | The app does not contain ads | None, and the manifest removes the advertising ID permission. |
| Advertising ID | The app does not use it | Same. |
| App access | All or some functionality is restricted: give the instructions below | A reviewer has no computer to pair with, and there is no account, so there is no login to give. The demo mode opens every screen. |
| Government app, financial features, health apps, news app, COVID-19 app | None apply | The app is none of them. |
| Account deletion URL | Not asked | The app has no account, so the "app lets users create an account" question is answered No. |

### App access instructions

Paste this into the instructions field, as one set named "Demo", kept short because the field is small:

```
No login. The app pairs with the user's own computer, which a reviewer does not have, so open it and tap "Try the demo" on the first screen (Korean: "체험해 보기"). It opens every screen with sample data: Agents, Missions, PRs, Settings. Nothing goes to a computer. Contact: hello@withhide.dev
```

The longer reviewer note the App Store uses is `mobile/store/metadata/review_information/notes.txt`; its demo steps (agents, mission, pull requests, "To look at", Settings) hold for the Android app too, except that Android asks its notification permission only when the person turns the Notifications switch on.

## Countries

All countries and regions Play offers, except the European Union and the European Economic Area.
The reason is the same as the iOS app's: this release is not distributed in the European Union.

## Store settings

| Field | Value |
| --- | --- |
| App or game | App |
| Free or paid | Free (this cannot be changed to paid later) |
| Category | Productivity |
| Contact email | hello@withhide.dev |
| Website | https://withhide.dev |
| Privacy policy | https://withhide.dev/privacy |
| Default language | English (United States) en-US, with Korean (Korea) ko-KR added |

The privacy policy must say what the notification registration is and who sees it before the app is submitted; that edit belongs to the site node, not to this folder.
