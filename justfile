# KishMat Chess Engine

# Default target
default: build

# Build in debug mode
build:
    cargo build --workspace

# Build optimized release binary (fat LTO + native CPU)
release:
    RUSTFLAGS="-C target-cpu=native" cargo build --release

# Run all tests (16MB stack for deep search recursion)
test:
    RUST_MIN_STACK=16777216 cargo test --workspace

# Run the engine in UCI mode
run:
    cargo run --release -- uci

# Run the engine in interactive play mode
play:
    cargo run --release -- play -d 8

# Analyze a FEN position
analyze fen depth="10":
    cargo run --release -- analyze -f "{{fen}}" -d {{depth}}

# Run perft test
perft depth="6":
    cargo run --release -- perft -d {{depth}}

# Run the ELO benchmark suite — auto-detects all hardware
bench:
    @echo "Running KishMat Benchmark Suite (all cores)..."
    RUSTFLAGS="-C target-cpu=native" cargo run --release -- bench

# Run a quick NPS benchmark at depth 16
nps:
    @echo "NPS Benchmark (depth 16, startpos)..."
    @printf 'uci\nisready\nposition startpos\ngo depth 16\nquit\n' | cargo run --release 2>&1 | tail -5

# Build and run the GUI application
ui:
    @echo "Building KishMat GUI..."
    cargo build --release -p kishmat-ui
    @echo "Launching KishMat Chess GUI..."
    cargo run --release -p kishmat-ui

# Build the updater
updater:
    cargo build --release -p kishmat-updater

# Run the updater (check for updates)
check-updates:
    cargo run --release -p kishmat-updater -- check

# Run the updater (update all components)
update:
    cargo run --release -p kishmat-updater -- update all

# Profile-guided optimization build (optional, requires nightly Rust)
pgo:
    @echo "Building with Profile-Guided Optimization (needs nightly)..."
    @echo "  Pass 1: Instrumented build..."
    RUSTFLAGS="-C target-cpu=native -Cprofile-generate=/tmp/kishmat-pgo" cargo +nightly build --release
    @echo "  Collecting profile data..."
    @printf 'uci\nisready\nposition startpos\ngo depth 14\nquit\n' | ./target/release/kishmat 2>&1 > /dev/null
    @echo "  Pass 2: Optimized build with PGO data..."
    RUSTFLAGS="-C target-cpu=native -Cprofile-use=/tmp/kishmat-pgo" cargo +nightly build --release
    @echo "  PGO build complete!"

# Clean build artifacts
clean:
    cargo clean

# Format all code
fmt:
    cargo fmt --all

# Run clippy lints
lint:
    cargo clippy --workspace -- -D warnings

# Check without building
check:
    cargo check --workspace

# Install KishMat locally (no root required)
# - Binaries go to ~/.local/bin (macOS/Linux) or %LOCALAPPDATA%/KishMat/bin (Windows)
# - macOS: creates .app bundle in ~/Applications with icon
# - Linux: creates .desktop file with icon in ~/.local/share
install:
    #!/usr/bin/env bash
    set -euo pipefail

    echo "🔨 Building KishMat (release)..."
    cargo build --release -p kishmat-ui
    cargo build --release

    # Detect OS
    OS="$(uname -s)"
    case "$OS" in
        Darwin)
            echo "🍎 Installing for macOS..."

            # Binaries
            mkdir -p "$HOME/.local/bin"
            cp target/release/kishmat "$HOME/.local/bin/kishmat"
            cp target/release/kishmat-ui "$HOME/.local/bin/kishmat-ui"
            chmod +x "$HOME/.local/bin/kishmat" "$HOME/.local/bin/kishmat-ui"

            # Ensure ~/.local/bin is in PATH
            if ! echo "$PATH" | grep -q "$HOME/.local/bin"; then
                echo ""
                echo "⚠️  Add ~/.local/bin to your PATH:"
                echo '  echo '\''export PATH="$HOME/.local/bin:$PATH"'\'' >> ~/.zshrc'
            fi

            # Create .app bundle in ~/Applications
            APP_DIR="$HOME/Applications/KishMat.app"
            mkdir -p "$APP_DIR/Contents/MacOS"
            mkdir -p "$APP_DIR/Contents/Resources"

            # Copy binary
            cp target/release/kishmat-ui "$APP_DIR/Contents/MacOS/KishMat"
            chmod +x "$APP_DIR/Contents/MacOS/KishMat"

            # Copy icon (PNG — macOS will use it; for full .icns, use sips)
            cp logo-nobg.png "$APP_DIR/Contents/Resources/icon.png"

            # Try to create .icns from PNG (macOS has sips built-in)
            if command -v sips &>/dev/null && command -v iconutil &>/dev/null; then
                ICONSET="$APP_DIR/Contents/Resources/icon.iconset"
                mkdir -p "$ICONSET"
                for sz in 16 32 64 128 256 512; do
                    sips -z $sz $sz logo-nobg.png --out "$ICONSET/icon_${sz}x${sz}.png" &>/dev/null
                    double=$((sz * 2))
                    sips -z $double $double logo-nobg.png --out "$ICONSET/icon_${sz}x${sz}@2x.png" &>/dev/null
                done
                iconutil -c icns "$ICONSET" -o "$APP_DIR/Contents/Resources/KishMat.icns" 2>/dev/null || true
                rm -rf "$ICONSET"
            fi

            ICNS_REF="KishMat"
            if [ -f "$APP_DIR/Contents/Resources/KishMat.icns" ]; then
                ICNS_REF="KishMat"
            fi

            # Info.plist
            cat > "$APP_DIR/Contents/Info.plist" <<PLIST
    <?xml version="1.0" encoding="UTF-8"?>
    <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
    <plist version="1.0">
    <dict>
        <key>CFBundleName</key>
        <string>KishMat</string>
        <key>CFBundleDisplayName</key>
        <string>KishMat Chess</string>
        <key>CFBundleIdentifier</key>
        <string>com.kishmat.chess</string>
        <key>CFBundleVersion</key>
        <string>2.0.0</string>
        <key>CFBundleShortVersionString</key>
        <string>2.0.0</string>
        <key>CFBundleExecutable</key>
        <string>KishMat</string>
        <key>CFBundleIconFile</key>
        <string>${ICNS_REF}</string>
        <key>CFBundlePackageType</key>
        <string>APPL</string>
        <key>NSHighResolutionCapable</key>
        <true/>
    </dict>
    </plist>
    PLIST

            echo "✅ Installed:"
            echo "   CLI engine: ~/.local/bin/kishmat"
            echo "   GUI app:    ~/Applications/KishMat.app"
            ;;

        Linux)
            echo "🐧 Installing for Linux..."

            # Binaries
            mkdir -p "$HOME/.local/bin"
            cp target/release/kishmat "$HOME/.local/bin/kishmat"
            cp target/release/kishmat-ui "$HOME/.local/bin/kishmat-ui"
            chmod +x "$HOME/.local/bin/kishmat" "$HOME/.local/bin/kishmat-ui"

            # Ensure ~/.local/bin is in PATH
            if ! echo "$PATH" | grep -q "$HOME/.local/bin"; then
                echo ""
                echo "⚠️  Add ~/.local/bin to your PATH:"
                echo '  echo '\''export PATH="$HOME/.local/bin:$PATH"'\'' >> ~/.bashrc'
            fi

            # Icon
            mkdir -p "$HOME/.local/share/icons/hicolor/256x256/apps"
            cp logo-nobg.png "$HOME/.local/share/icons/hicolor/256x256/apps/kishmat.png"

            # Desktop entry
            mkdir -p "$HOME/.local/share/applications"
            cat > "$HOME/.local/share/applications/kishmat.desktop" <<DESKTOP
    [Desktop Entry]
    Name=KishMat Chess
    Comment=The First Arabian Chess Engine
    Exec=$HOME/.local/bin/kishmat-ui
    Icon=kishmat
    Terminal=false
    Type=Application
    Categories=Game;BoardGame;
    Keywords=chess;engine;uci;
    DESKTOP

            echo "✅ Installed:"
            echo "   CLI engine: ~/.local/bin/kishmat"
            echo "   GUI:        ~/.local/bin/kishmat-ui"
            echo "   Desktop:    ~/.local/share/applications/kishmat.desktop"
            ;;

        MINGW*|MSYS*|CYGWIN*)
            echo "🪟 Installing for Windows..."

            INSTALL_DIR="$LOCALAPPDATA/KishMat/bin"
            mkdir -p "$INSTALL_DIR"
            cp target/release/kishmat.exe "$INSTALL_DIR/kishmat.exe"
            cp target/release/kishmat-ui.exe "$INSTALL_DIR/kishmat-ui.exe"
            cp logo-nobg.png "$INSTALL_DIR/icon.png"

            echo "✅ Installed to: $INSTALL_DIR"
            echo "⚠️  Add to PATH: Settings → System → Environment Variables → Path → Add: $INSTALL_DIR"
            ;;

        *)
            echo "❌ Unsupported OS: $OS"
            echo "   Manually copy target/release/kishmat and target/release/kishmat-ui to your PATH."
            exit 1
            ;;
    esac

    echo ""
    echo "🎉 KishMat installed successfully!"

# Uninstall KishMat
uninstall:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "Removing KishMat..."
    rm -f "$HOME/.local/bin/kishmat" "$HOME/.local/bin/kishmat-ui"
    rm -rf "$HOME/Applications/KishMat.app"
    rm -f "$HOME/.local/share/applications/kishmat.desktop"
    rm -f "$HOME/.local/share/icons/hicolor/256x256/apps/kishmat.png"
    echo "✅ KishMat uninstalled."
