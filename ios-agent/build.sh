#!/bin/bash
# Builds the qapilot iOS agent for one device or simulator:
#   ios-agent/build.sh <udid>
# → $QAPILOT_HOME (default ~/.qapilot)/ios-agent/<udid>/{Debug-<sdk>/QapilotAgent-Runner.app, agent.xctestrun}
# A real iPhone needs a development identity in the keychain and a wildcard provisioning profile (Xcode → Settings →
# Accounts) that lists the phone; both are found automatically. A simulator needs no signing.
set -euo pipefail
UDID="${1:?usage: build.sh <udid>}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="${QAPILOT_HOME:-$HOME/.qapilot}/ios-agent/$UDID"

if xcrun simctl list devices | grep -q "$UDID"; then SDK=iphonesimulator; TARGET=arm64-apple-ios17.0-simulator; else SDK=iphoneos; TARGET=arm64-apple-ios17.0; fi
SDKPATH="$(xcrun --sdk $SDK --show-sdk-path)"
PLAT="$(xcrun --sdk $SDK --show-sdk-platform-path)"
CONF="Debug-$SDK"
RUNNER="$OUT/$CONF/QapilotAgent-Runner.app"
HOSTAPP="$OUT/$CONF/QapilotHost.app"
BUNDLE="$RUNNER/PlugIns/QapilotAgent.xctest"
rm -rf "$OUT"; mkdir -p "$BUNDLE"

# 1. The test bundle (a Mach-O bundle that the runner loads).
xcrun --sdk $SDK swiftc -swift-version 5 -O -sdk "$SDKPATH" -target $TARGET -module-name QapilotAgent \
  -F "$PLAT/Developer/Library/Frameworks" -F "$PLAT/Developer/Library/PrivateFrameworks" \
  -I "$PLAT/Developer/usr/lib" -L "$PLAT/Developer/usr/lib" \
  -Xlinker -bundle -emit-executable -o "$BUNDLE/QapilotAgent" "$HERE/Agent.swift" \
  -framework XCTest -Xlinker -rpath -Xlinker @executable_path/Frameworks -Xlinker -rpath -Xlinker @loader_path/../../Frameworks
cat > "$BUNDLE/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>QapilotAgent</string>
<key>CFBundleIdentifier</key><string>com.qapilot.agent</string>
<key>CFBundleName</key><string>QapilotAgent</string>
<key>CFBundlePackageType</key><string>BNDL</string>
<key>CFBundleShortVersionString</key><string>1.0</string>
<key>CFBundleVersion</key><string>1</string>
<key>CFBundleSupportedPlatforms</key><array><string>$( [ $SDK = iphoneos ] && echo iPhoneOS || echo iPhoneSimulator )</string></array>
<key>MinimumOSVersion</key><string>17.0</string>
</dict></plist>
EOF

# 1b. The empty target app (Xcode requires one for a UI test bundle).
mkdir -p "$HOSTAPP"
xcrun --sdk $SDK swiftc -swift-version 5 -O -sdk "$SDKPATH" -target $TARGET -module-name QapilotHost -parse-as-library \
  -o "$HOSTAPP/QapilotHost" "$HERE/Host.swift"
cat > "$HOSTAPP/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>QapilotHost</string>
<key>CFBundleIdentifier</key><string>com.qapilot.host</string>
<key>CFBundleName</key><string>qapilot</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>1.0</string>
<key>CFBundleVersion</key><string>1</string>
<key>CFBundleSupportedPlatforms</key><array><string>$( [ $SDK = iphoneos ] && echo iPhoneOS || echo iPhoneSimulator )</string></array>
<key>LSRequiresIPhoneOS</key><true/>
<key>MinimumOSVersion</key><string>17.0</string>
<key>UIDeviceFamily</key><array><integer>1</integer><integer>2</integer></array>
<key>UILaunchScreen</key><dict/>
<key>UIRequiredDeviceCapabilities</key><array><string>arm64</string></array>
</dict></plist>
EOF

# 2. The runner: Xcode's XCTRunner with its placeholders filled, the XCTest frameworks next to it.
cp -R "$PLAT/Developer/Library/Xcode/Agents/XCTRunner.app/." "$RUNNER/"
# Thin to arm64: Xcode's runner is arm64 + arm64e, and an arm64e process cannot load the arm64 test bundle.
lipo -thin arm64 "$RUNNER/XCTRunner" -output "$RUNNER/QapilotAgent-Runner" && rm "$RUNNER/XCTRunner"
plutil -replace CFBundleIdentifier -string com.qapilot.agent.xctrunner "$RUNNER/Info.plist"
plutil -replace CFBundleName -string QapilotAgent-Runner "$RUNNER/Info.plist"
plutil -replace CFBundleExecutable -string QapilotAgent-Runner "$RUNNER/Info.plist"
mkdir -p "$RUNNER/Frameworks"
for f in XCTest XCUIAutomation Testing; do cp -R "$PLAT/Developer/Library/Frameworks/$f.framework" "$RUNNER/Frameworks/"; done
for f in XCTestCore XCTestSupport XCUnit XCTAutomationSupport; do cp -R "$PLAT/Developer/Library/PrivateFrameworks/$f.framework" "$RUNNER/Frameworks/"; done
cp "$PLAT/Developer/usr/lib/libXCTestSwiftSupport.dylib" "$RUNNER/Frameworks/"

# 3. Signing (a real iPhone only): a wildcard profile listing the phone + an identity whose certificate it contains.
if [ $SDK = iphoneos ]; then
  IFS=$'\t' read -r PROFILE TEAM IDENTITY < <(python3 -I - "$UDID" <<'PY'
import glob, hashlib, os, plistlib, subprocess, sys, datetime
udid = sys.argv[1]
ids = {}
for line in subprocess.run(['security', 'find-identity', '-v', '-p', 'codesigning'], capture_output=True, text=True).stdout.splitlines():
    parts = line.split()
    if len(parts) > 2 and len(parts[1]) == 40:
        ids[parts[1]] = True
dirs = [os.path.expanduser('~/Library/Developer/Xcode/UserData/Provisioning Profiles'), os.path.expanduser('~/Library/MobileDevice/Provisioning Profiles')]
for f in sorted(p for d in dirs for p in glob.glob(d + '/*.mobileprovision')):
    raw = subprocess.run(['security', 'cms', '-D', '-i', f], capture_output=True).stdout
    try:
        p = plistlib.loads(raw)
    except Exception:
        continue
    app = p.get('Entitlements', {}).get('application-identifier', '')
    if not app.endswith('.*') or udid not in (p.get('ProvisionedDevices') or []):
        continue
    if p['ExpirationDate'] < datetime.datetime.now():
        continue
    for c in p.get('DeveloperCertificates', []):
        h = hashlib.sha1(c).hexdigest().upper()
        if h in ids:
            print(f, p['TeamIdentifier'][0], h, sep='\t')
            sys.exit(0)
sys.exit('no wildcard provisioning profile lists this iPhone with a certificate in the keychain (Xcode → Settings → Accounts → download profiles)')
PY
)
  sign() { codesign -f -s "$IDENTITY" --timestamp=none "$@" 2>&1 | grep -v "replacing existing signature" || true; }
  ENT="$OUT/entitlements.plist"
  cat > "$ENT" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>application-identifier</key><string>$TEAM.com.qapilot.agent.xctrunner</string>
<key>com.apple.developer.team-identifier</key><string>$TEAM</string>
<key>get-task-allow</key><true/>
<key>keychain-access-groups</key><array><string>$TEAM.*</string></array>
</dict></plist>
EOF
  cp "$PROFILE" "$RUNNER/embedded.mobileprovision"
  cp "$PROFILE" "$HOSTAPP/embedded.mobileprovision"
  sed "s/com.qapilot.agent.xctrunner/com.qapilot.host/" "$ENT" > "$OUT/entitlements-host.plist"
  sign --entitlements "$OUT/entitlements-host.plist" "$HOSTAPP"
  for f in "$RUNNER/Frameworks/"*; do sign "$f"; done
  sign "$BUNDLE"
  sign --entitlements "$ENT" "$RUNNER"
  codesign --verify --deep --strict "$RUNNER"
  echo "signed for team $TEAM with $(basename "$PROFILE")"
else
  codesign -f -s - --timestamp=none "$BUNDLE" "$RUNNER" "$HOSTAPP" 2>/dev/null || true
fi

# 4. The xctestrun that xcodebuild test-without-building runs.
PLATNAME=$( [ $SDK = iphoneos ] && echo iPhoneOS || echo iPhoneSimulator )
cat > "$OUT/agent.xctestrun" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>QapilotAgent</key><dict>
  <key>BlueprintName</key><string>QapilotAgent</string>
  <key>IsUITestBundle</key><true/>
  <key>IsXCTRunnerHostedTestBundle</key><true/>
  <key>TestBundlePath</key><string>__TESTHOST__/PlugIns/QapilotAgent.xctest</string>
  <key>TestHostPath</key><string>__TESTROOT__/$CONF/QapilotAgent-Runner.app</string>
  <key>TestHostBundleIdentifier</key><string>com.qapilot.agent.xctrunner</string>
  <key>ProductModuleName</key><string>QapilotAgent</string>
  <key>UITargetAppPath</key><string>__TESTROOT__/$CONF/QapilotHost.app</string>
  <key>DependentProductPaths</key><array><string>__TESTROOT__/$CONF/QapilotHost.app</string><string>__TESTROOT__/$CONF/QapilotAgent-Runner.app</string></array>
  <key>TestingEnvironmentVariables</key><dict>$( [ $SDK = iphonesimulator ] && echo "
    <key>DYLD_FRAMEWORK_PATH</key><string>__TESTROOT__/$CONF:__PLATFORMS__/$PLATNAME.platform/Developer/Library/Frameworks</string>
    <key>DYLD_LIBRARY_PATH</key><string>__TESTROOT__/$CONF:__PLATFORMS__/$PLATNAME.platform/Developer/Library/Frameworks</string>" )
  </dict>
  <key>EnvironmentVariables</key><dict/>
  <key>CommandLineArguments</key><array/>
  <key>TestTimeoutsEnabled</key><false/>
  <key>SystemAttachmentLifetime</key><string>deleteOnSuccess</string>
  <key>UserAttachmentLifetime</key><string>deleteOnSuccess</string>
</dict>
<key>__xctestrun_metadata__</key><dict><key>FormatVersion</key><integer>1</integer></dict>
</dict></plist>
EOF
echo "$OUT/agent.xctestrun"
