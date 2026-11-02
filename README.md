# Finance Tracker (mobile)

A native Rust port of the [Finance Tracker web app](https://github.com/SpeakingInBits/FinanceManager)
for iOS and Android (it also runs on Windows, macOS and Linux for development).

Same features as the web app: income/expense tracking, recurring transactions (monthly and yearly,
with history kept when an amount changes), a two-level category tree, monthly and one-time budgets
with fills and withdrawals, a dashboard, and pie and Sankey charts. Everything stays on the device;
there are no accounts and no network access.

## Why it's light and fast

- **No web view, no JavaScript.** The UI is [Slint](https://slint.dev), compiled to native code and
  drawn on the GPU (OpenGL ES on Android, Skia/Metal on iOS).
- **One small data file.** Data is a single JSON document in the app's private storage, loaded into
  memory at start-up and written atomically (temp file + rename) after each change.
- **Charts are computed in Rust** (a port of `d3-sankey`'s layout and `d3.arc`) and drawn as vector
  paths, so there is no charting runtime.

## Project layout

| Path | What it is |
| --- | --- |
| `core/` | `finance-core`: data model, persistence, recurrence, budget maths, chart aggregation and geometry. No UI dependencies; all the business logic and its tests live here. |
| `app/` | `finance-app`: the Slint UI (`app/ui/*.slint`) and the Rust controller that connects it to `core` (`app/src`). |
| `app/android/` | Gradle project that packages the Rust library into an APK or Play Store app bundle. |
| `app/ios/` | XcodeGen spec, app icon and the build script Xcode runs to compile the app with cargo. |

How the web app maps to this one:

| Web (TypeScript) | Here (Rust) |
| --- | --- |
| `src/models/*` | `core/src/model.rs` (same field names, so records serialize the same way) |
| `src/db/*` (IndexedDB) | `core/src/store.rs` (JSON file) |
| `src/state/actions.ts` | `core/src/store.rs` (`Store` methods) and `app/src/controller.rs` |
| `src/utils/*` | `core/src/{date,currency,recurrence,budget}.rs` |
| `src/charts/chart-utils.ts` | `core/src/charts.rs` |
| D3 pie / `d3-sankey` | `core/src/geometry.rs` |
| Web components | `app/ui/*.slint` |

## Development

Requires Rust 1.92 or later.

```bash
cargo run -p finance-app
```

Runs the app in a desktop window sized like a phone. Data goes to your user data folder
(`%APPDATA%\FinanceTracker` on Windows); point it elsewhere for testing with:

```bash
cargo run -p finance-app -- --data-dir ./scratch-data
```

```bash
cargo test --workspace
```

```bash
cargo clippy --workspace --all-targets
```

The UI uses the Slint language; the [Slint VS Code extension](https://marketplace.visualstudio.com/items?itemName=Slint.slint)
gives live preview of `.slint` files.

## Building for Android

One-time setup:

1. Install [Android Studio](https://developer.android.com/studio). In **Settings → Languages &
   Frameworks → Android SDK**, install an SDK platform (API 36), and under **SDK Tools** the
   **NDK** and **Android SDK Command-line Tools**.
2. Set `ANDROID_HOME` to the SDK folder and `ANDROID_NDK_HOME` to the NDK folder inside it (for
   example `%LOCALAPPDATA%\Android\Sdk` and `%LOCALAPPDATA%\Android\Sdk\ndk\<version>`).
3. Install the Rust targets and cargo-ndk:

   ```bash
   rustup target add aarch64-linux-android x86_64-linux-android
   ```

   ```bash
   cargo install cargo-ndk
   ```

Then open `app/android` in Android Studio and press Run (the Gradle build calls cargo-ndk for you),
or from the command line in `app/android`, with `JAVA_HOME` pointing at JDK 17 or later (Android
Studio's bundled JDK works):

```bash
./gradlew installDebug
```

The build always compiles the Rust code in release mode, for both `arm64-v8a` (phones) and
`x86_64` (emulators). The libraries are aligned to 16 KB pages, as Google Play requires.

Debug builds let you read or replace the app's data file over adb, which is handy for testing:

```bash
adb shell run-as com.speakinginbits.financetracker cat files/finance-tracker.json
```

For the Play Store, build a signed app bundle with **Build → Generate Signed App Bundle** in Android
Studio, or `./gradlew bundleRelease` with a signing config. The bundle is
`app/android/app/build/outputs/bundle/release/app-release.aab`.

The app id is `com.speakinginbits.financetracker`; change `applicationId` in
`app/android/app/build.gradle.kts` before the first upload if you want a different one. It can't
change after publishing.

## Building for iOS

Needs a Mac with Xcode and [XcodeGen](https://github.com/yonaskolb/XcodeGen).

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
```

```bash
cd app/ios && xcodegen
```

Open the generated `FinanceTracker.xcodeproj`, choose your team under **Signing & Capabilities**,
and run on a simulator or device. Xcode runs `build_for_ios_with_cargo.bash`, which builds the
`finance-tracker` binary with cargo. For the App Store, use **Product → Archive**.

The bundle id is `com.speakinginbits.financetracker` (set in `app/ios/project.yml`).

## Releases

Run the **Release** workflow from the repository's **Actions** tab (**Run workflow**, from `main`).
Choose what the release contains (bug fixes, new features or breaking changes) and it picks the next
version number from the latest `vX.Y.Z` tag, or type a version to use instead. It builds the Android
APK and app bundle and an iOS `.ipa` (always unsigned for now), then publishes a GitHub release with them and the
install instructions in `packaging/release-notes.md`. Pull requests that change the Android or iOS
projects, `packaging/` or the workflow build the packages without releasing.

The Android packages are signed with the Play upload key from these repository secrets:

| Secret | Value |
| --- | --- |
| `ANDROID_KEYSTORE_BASE64` | The keystore file, base64-encoded |
| `ANDROID_KEYSTORE_PASSWORD` | The keystore's password |
| `ANDROID_KEY_ALIAS` | The key's alias |
| `ANDROID_KEY_PASSWORD` | The key's password (leave unset if it's the keystore's password) |

Without them the Android packages are unsigned and named `FinanceTracker-unsigned.apk` and
`FinanceTracker-unsigned.aab`; Android won't install them until they're signed. Keep the keystore
somewhere safe: Android only installs updates signed with the same key.

To make a keystore and copy it base64-encoded (PowerShell):

```powershell
keytool -genkeypair -keystore upload.jks -alias upload -keyalg RSA -keysize 2048 -validity 10000
[Convert]::ToBase64String([IO.File]::ReadAllBytes("upload.jks")) | Set-Clipboard
```

## Your data

Data is stored in a single file, `finance-tracker.json`, in the app's private storage: internal
storage on Android, `Library/Application Support` in the app container on iOS. Android cloud backup
is turned off so data never leaves the device. Uninstalling the app deletes the data.

The file uses the same record shapes and field names as the web app's IndexedDB stores, so data
can be moved between the two once an import/export feature exists. A file that can't be read is
renamed to `finance-tracker.corrupt.json` rather than overwritten.

## Differences from the web app

- **Navigation:** the page title and **Add** button live in the top bar; forms open as bottom
  sheets; the Android back button closes the open sheet or dialog.
- **Charts on touch screens:** tap a pie slice to see its name and amount in the middle of the
  donut; tap a Sankey bar to see its total.
- **Category colors:** chosen from a palette of twelve swatches instead of a free color picker.
- **Date picker and Android status bar:** follow the phone's light/dark setting even when the
  app's theme is overridden, so on a light-mode phone with the app set to dark, the status-bar
  icons stay dark.
- **Budget nodes** in the Sankey chart have a solid outline instead of a dashed one.
