#!/usr/bin/env bash
# Build target/apk/foqdt.apk (arm64-v8a) with the SDK command-line tools. No Gradle needed.
# Needs: Android SDK (build-tools, a platform, NDK), JDK, zip, rustup targets aarch64-linux-android and x86_64-linux-android.
set -euo pipefail
cd "$(dirname "$0")"
ROOT=$(cd ../.. && pwd)
SDK=${ANDROID_HOME:-$HOME/Android/Sdk}
NDK=${ANDROID_NDK_HOME:-$(ls -d "$SDK"/ndk/* | sort -V | tail -1)}
BT=$(ls -d "$SDK"/build-tools/* | sort -V | tail -1)
JAR=$(ls -d "$SDK"/platforms/android-* | sort -V | tail -1)/android.jar
API=26
TC=$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin

# Phones only by default; ABIS="arm64-v8a x86_64" adds the emulator library.
OUT=$ROOT/target/apk
rm -rf "$OUT" && mkdir -p "$OUT/classes"
for ABI in ${ABIS:-arm64-v8a}; do
    case $ABI in
        arm64-v8a) TRIPLE=aarch64-linux-android ;;
        x86_64) TRIPLE=x86_64-linux-android ;;
        *) echo "unsupported ABI $ABI" >&2; exit 1 ;;
    esac
    VAR=$(echo "$TRIPLE" | tr 'a-z-' 'A-Z_')
    export "CARGO_TARGET_${VAR}_LINKER=$TC/$TRIPLE$API-clang"
    export "CC_${TRIPLE//-/_}=$TC/$TRIPLE$API-clang"
    export "AR_${TRIPLE//-/_}=$TC/llvm-ar"
    export "CXX_${TRIPLE//-/_}=$TC/$TRIPLE$API-clang++"   # zxing-cpp is C++
    export ANDROID_NDK_ROOT=$NDK
    cargo build --release -p foqdt-android --target "$TRIPLE"
    mkdir -p "$OUT/root/lib/$ABI"
    cp "$ROOT/target/$TRIPLE/release/libfoqdt_android.so" "$OUT/root/lib/$ABI/"
    "$TC/llvm-strip" "$OUT/root/lib/$ABI/libfoqdt_android.so"
done

javac --release 11 -nowarn -cp "$JAR" -d "$OUT/classes" java/dev/foqdt/*.java
"$BT/d8" --release --min-api $API --lib "$JAR" --output "$OUT/root" $(find "$OUT/classes" -name '*.class')

"$BT/aapt2" compile --dir res -o "$OUT/res.zip"
"$BT/aapt2" link -o "$OUT/unsigned.apk" -I "$JAR" --manifest AndroidManifest.xml -R "$OUT/res.zip" --auto-add-overlay
(cd "$OUT/root" && zip -qr "$OUT/unsigned.apk" classes.dex lib)
"$BT/zipalign" -f -P 16 4 "$OUT/unsigned.apk" "$OUT/aligned.apk"

KS=${KEYSTORE:-$HOME/.android/debug.keystore}
if [ ! -f "$KS" ]; then
    mkdir -p "$(dirname "$KS")"
    keytool -genkeypair -keystore "$KS" -storepass android -keypass android -alias androiddebugkey \
        -dname "CN=Android Debug,O=Android,C=US" -keyalg RSA -keysize 2048 -validity 10000
fi
"$BT/apksigner" sign --ks "$KS" --ks-pass pass:${KEYSTORE_PASS:-android} --out "$OUT/foqdt.apk" "$OUT/aligned.apk"
echo "$OUT/foqdt.apk"
