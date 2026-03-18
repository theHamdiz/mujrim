# KishMat Chess Engine

# Default target
default: build

# Build in debug mode
build:
    cargo build --workspace

# Build optimized release binaries for ALL crates (native CPU)
release:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "🔨 Building all KishMat crates (optimized release, native CPU)..."
    RUSTFLAGS="-C target-cpu=native" cargo build --release --workspace
    echo ""
    echo "✅ Release binaries:"
    for bin in kishmat kishmat-ui kishmat-updater; do
        [ -f "target/release/$bin" ] && echo "   target/release/$bin"
    done

# Release build for macOS (aarch64 + x86_64)
release-darwin:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "🍎 Building for macOS (aarch64 + x86_64)..."
    DIST="dist/darwin"
    rm -rf "$DIST"
    mkdir -p "$DIST/aarch64" "$DIST/x86_64"

    # aarch64 (Apple Silicon)
    echo "  → aarch64-apple-darwin..."
    RUSTFLAGS="-C target-cpu=apple-m1" cargo build --release --workspace --target aarch64-apple-darwin
    for bin in kishmat kishmat-ui kishmat-updater; do
        [ -f "target/aarch64-apple-darwin/release/$bin" ] && \
            cp "target/aarch64-apple-darwin/release/$bin" "$DIST/aarch64/"
    done

    # x86_64 (Intel) — only if target is installed
    if rustup target list --installed | grep -q x86_64-apple-darwin; then
        echo "  → x86_64-apple-darwin..."
        cargo build --release --workspace --target x86_64-apple-darwin
        for bin in kishmat kishmat-ui kishmat-updater; do
            [ -f "target/x86_64-apple-darwin/release/$bin" ] && \
                cp "target/x86_64-apple-darwin/release/$bin" "$DIST/x86_64/"
        done
    else
        echo "  ⚠️  x86_64-apple-darwin not installed. Run: rustup target add x86_64-apple-darwin"
    fi

    echo ""
    echo "✅ macOS release:"
    find "$DIST" -type f -perm +111 | sort | while read f; do echo "   $f"; done

# Release build for Linux (x86_64 + aarch64)
release-linux:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "🐧 Building for Linux..."
    DIST="dist/linux"
    rm -rf "$DIST"
    mkdir -p "$DIST/x86_64" "$DIST/aarch64"

    # x86_64
    if rustup target list --installed | grep -q x86_64-unknown-linux-gnu; then
        echo "  → x86_64-unknown-linux-gnu..."
        cargo build --release --workspace --target x86_64-unknown-linux-gnu 2>&1 || \
            echo "  ⚠️  x86_64-unknown-linux-gnu build failed (may need cross-linker)"
        for bin in kishmat kishmat-ui kishmat-updater; do
            [ -f "target/x86_64-unknown-linux-gnu/release/$bin" ] && \
                cp "target/x86_64-unknown-linux-gnu/release/$bin" "$DIST/x86_64/"
        done
    else
        echo "  ⚠️  x86_64-unknown-linux-gnu not installed. Run: rustup target add x86_64-unknown-linux-gnu"
    fi

    # aarch64
    if rustup target list --installed | grep -q aarch64-unknown-linux-gnu; then
        echo "  → aarch64-unknown-linux-gnu..."
        cargo build --release --workspace --target aarch64-unknown-linux-gnu 2>&1 || \
            echo "  ⚠️  aarch64-unknown-linux-gnu build failed (may need cross-linker)"
        for bin in kishmat kishmat-ui kishmat-updater; do
            [ -f "target/aarch64-unknown-linux-gnu/release/$bin" ] && \
                cp "target/aarch64-unknown-linux-gnu/release/$bin" "$DIST/aarch64/"
        done
    else
        echo "  ⚠️  aarch64-unknown-linux-gnu not installed. Run: rustup target add aarch64-unknown-linux-gnu"
    fi

    echo ""
    echo "✅ Linux release:"
    find "$DIST" -type f 2>/dev/null | sort | while read f; do echo "   $f"; done

# Release build for Windows (x86_64, requires cargo-xwin or cross)
release-win:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "🪟 Building for Windows (x86_64)..."
    DIST="dist/windows/x86_64"
    rm -rf "$DIST"
    mkdir -p "$DIST"

    TARGET="x86_64-pc-windows-gnu"

    # Check target is installed
    if ! rustup target list --installed | grep -q "$TARGET"; then
        echo "  Installing target $TARGET..."
        rustup target add "$TARGET"
    fi

    # Try cargo-xwin first (better Windows SDK support on non-Windows hosts)
    if command -v cargo-xwin &>/dev/null; then
        echo "  → Using cargo-xwin for $TARGET..."
        cargo xwin build --release --workspace --target "$TARGET"
    else
        echo "  → Using cargo build for $TARGET..."
        echo "  (Install cargo-xwin for better results: cargo install cargo-xwin)"
        cargo build --release --workspace --target "$TARGET" 2>&1 || \
            echo "  ⚠️  Build failed. Install a mingw-w64 cross-compiler or cargo-xwin."
    fi

    for bin in kishmat kishmat-ui kishmat-updater; do
        [ -f "target/$TARGET/release/$bin.exe" ] && \
            cp "target/$TARGET/release/$bin.exe" "$DIST/"
    done

    echo ""
    echo "✅ Windows release:"
    find "$DIST" -type f 2>/dev/null | sort | while read f; do echo "   $f"; done

# Release build for ALL platforms
release-full: release-darwin release-linux release-win
    #!/usr/bin/env bash
    echo ""
    echo "🌍 Full cross-platform release complete!"
    echo ""
    echo "📦 Distribution layout:"
    find dist -type f 2>/dev/null | sort | while read f; do echo "   $f"; done

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

# ──────────────────────────────────────────────────────────────
# NNUE Network Adapter Variants
# ──────────────────────────────────────────────────────────────

# Build with ALL NNUE adapters (Akimbo + Stockfish) — default
build-full:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "🧠 Building with all NNUE adapters (Akimbo + Stockfish)..."
    RUSTFLAGS="-C target-cpu=native" cargo build --release
    echo "✅ Full build complete: target/release/kishmat"

# Build with Akimbo-family adapter only (no Stockfish .nnue support)
build-akimbo:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "🧠 Building with Akimbo NNUE adapter only..."
    RUSTFLAGS="-C target-cpu=native" cargo build --release --no-default-features \
        --features "mimalloc,trainer,gpu,xboard,book,nnue,simd,akimbo-nnue"
    echo "✅ Akimbo-only build complete: target/release/kishmat"

# Build with Stockfish adapter only (no external Akimbo loader)
build-stockfish:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "🧠 Building with Stockfish NNUE adapter only..."
    RUSTFLAGS="-C target-cpu=native" cargo build --release --no-default-features \
        --features "mimalloc,trainer,gpu,xboard,book,nnue,simd,stockfish-nnue"
    echo "✅ Stockfish-only build complete: target/release/kishmat"

# Build with embedded NNUE only (no external network loading)
build-embedded:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "🧠 Building with embedded NNUE only (no adapters)..."
    RUSTFLAGS="-C target-cpu=native" cargo build --release --no-default-features \
        --features "mimalloc,trainer,gpu,xboard,book,nnue,simd"
    echo "✅ Embedded-only build complete: target/release/kishmat"

# Build minimal engine (no book, no adapters, no GUI extras)
build-minimal:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "🧠 Building minimal engine (NNUE only, no adapters/book)..."
    RUSTFLAGS="-C target-cpu=native" cargo build --release -p kishmat --no-default-features \
        --features "nnue,simd"
    echo "✅ Minimal build complete: target/release/kishmat"

# List all available NNUE build variants
build-variants:
    @echo "Available NNUE build variants:"
    @echo "  just build-full       — All adapters (Akimbo + Stockfish) [default]"
    @echo "  just build-akimbo     — Akimbo-family adapter only"
    @echo "  just build-stockfish  — Stockfish .nnue adapter only"
    @echo "  just build-embedded   — Embedded net only (no external loading)"
    @echo "  just build-minimal    — Minimal engine (no book, no adapters)"
    @echo ""
    @echo "Network management:"
    @echo "  just nets             — Download ALL latest networks"
    @echo "  just net-stockfish    — Download latest Stockfish networks"
    @echo "  just net-akimbo       — Download latest Akimbo network"
    @echo "  just net-viridithas   — Download latest Viridithas network"
    @echo "  just net-status       — Show downloaded networks"
    @echo ""
    @echo "Build + bench with specific net:"
    @echo "  just bench-net <file> — Bench with an external .nnue / .bin network"

# ──────────────────────────────────────────────────────────────
# NNUE Network Downloads
# Download latest networks from top open-source engines.
# All networks are saved to crates/kishmat-eval/resources/
# and excluded from Git via .gitignore.
# ──────────────────────────────────────────────────────────────

# Directory for all downloaded networks
nets_dir := "crates/kishmat-eval/resources"

# Download ALL latest NNUE networks from supported engines
nets: net-stockfish net-akimbo net-viridithas
    #!/usr/bin/env bash
    echo ""
    echo "✅ All networks downloaded!"
    just net-status

# Download latest Stockfish NNUE networks (big + small)
net-stockfish:
    #!/usr/bin/env bash
    set -euo pipefail
    DIR="{{nets_dir}}"
    mkdir -p "$DIR"
    echo "🐟 Downloading Stockfish 18 networks..."

    # Get net names from Stockfish sf_18 release tag (evaluate.h)
    EVAL_H=$(curl -sL "https://raw.githubusercontent.com/official-stockfish/Stockfish/sf_18/src/evaluate.h")
    BIG=$(echo "$EVAL_H" | grep -o 'nn-[a-f0-9]\{12\}\.nnue' | head -1)
    SMALL=$(echo "$EVAL_H" | grep -o 'nn-[a-f0-9]\{12\}\.nnue' | tail -1)

    if [ -z "$BIG" ]; then
        echo "  ⚠️  Could not detect Stockfish big net name. Using fallback."
        BIG="nn-c288c895ea92.nnue"
    fi

    # Download big net
    if [ -f "$DIR/$BIG" ]; then
        echo "  ✓ $BIG already exists ($(du -h "$DIR/$BIG" | cut -f1))"
    else
        echo "  → Downloading $BIG..."
        curl -sL -o "$DIR/$BIG" "https://tests.stockfishchess.org/api/nn/$BIG"
        echo "  ✓ $BIG ($(du -h "$DIR/$BIG" | cut -f1))"
    fi

    # Download small net (if different from big)
    if [ -n "$SMALL" ] && [ "$SMALL" != "$BIG" ]; then
        if [ -f "$DIR/$SMALL" ]; then
            echo "  ✓ $SMALL already exists ($(du -h "$DIR/$SMALL" | cut -f1))"
        else
            echo "  → Downloading $SMALL..."
            curl -sL -o "$DIR/$SMALL" "https://tests.stockfishchess.org/api/nn/$SMALL"
            echo "  ✓ $SMALL ($(du -h "$DIR/$SMALL" | cut -f1))"
        fi
    fi

# Download latest Akimbo NNUE network
net-akimbo:
    #!/usr/bin/env bash
    set -euo pipefail
    DIR="{{nets_dir}}"
    mkdir -p "$DIR"
    echo "🎯 Downloading Akimbo network (latest release)..."

    # Try GitHub releases API for jw1912/akimbo
    URL=$(curl -sL "https://api.github.com/repos/jw1912/akimbo/releases/latest" \
          | grep -o '"browser_download_url":\s*"[^"]*net\.bin"' \
          | head -1 \
          | sed 's/.*"browser_download_url":\s*"//' | sed 's/"$//')

    if [ -n "$URL" ]; then
        FNAME="akimbo-latest.bin"
        echo "  → Downloading from: $URL"
        curl -sL -o "$DIR/$FNAME" "$URL"
        echo "  ✓ $FNAME ($(du -h "$DIR/$FNAME" | cut -f1))"
    else
        echo "  ℹ️  No standalone net.bin in Akimbo releases."
        echo "     The embedded net.bin (6MB, 768→1024×2→1) is already compiled in."
        if [ -f "$DIR/net.bin" ]; then
            echo "  ✓ net.bin already present ($(du -h "$DIR/net.bin" | cut -f1))"
        fi
    fi

# Download latest Viridithas NNUE network
net-viridithas:
    #!/usr/bin/env bash
    set -euo pipefail
    DIR="{{nets_dir}}"
    mkdir -p "$DIR"
    echo "🔮 Downloading Viridithas network (latest release)..."

    # Get download URL from GitHub releases API
    URL=$(curl -sL "https://api.github.com/repos/cosmobobak/viridithas-networks/releases/latest" \
          | grep -o '"browser_download_url":\s*"[^"]*"' \
          | head -1 \
          | sed 's/"browser_download_url":\s*"//' | sed 's/"$//')

    if [ -z "$URL" ]; then
        echo "  ⚠️  Could not find Viridithas network release."
        exit 0
    fi

    FNAME=$(basename "$URL")
    echo "  → Downloading $FNAME..."
    curl -sL -o "$DIR/$FNAME" "$URL"

    # Decompress if .zst
    if echo "$FNAME" | grep -q '\.zst$'; then
        if command -v zstd &>/dev/null; then
            echo "  → Decompressing $FNAME..."
            zstd -d --rm -f "$DIR/$FNAME" 2>/dev/null
            DECOMPRESSED="${FNAME%.zst}"
            echo "  ✓ $DECOMPRESSED ($(du -h "$DIR/$DECOMPRESSED" | cut -f1))"
        else
            echo "  ⚠️  Install zstd to decompress: brew install zstd"
            echo "  ✓ $FNAME (compressed, $(du -h "$DIR/$FNAME" | cut -f1))"
        fi
    else
        echo "  ✓ $FNAME ($(du -h "$DIR/$FNAME" | cut -f1))"
    fi

# Show all downloaded NNUE networks
net-status:
    #!/usr/bin/env bash
    DIR="{{nets_dir}}"
    echo "📦 NNUE Networks in $DIR:"
    echo ""
    if [ -d "$DIR" ]; then
        for f in "$DIR"/*; do
            [ -f "$f" ] || continue
            name=$(basename "$f")
            size=$(du -h "$f" | cut -f1)
            case "$name" in
                nn-*.nnue)  engine="Stockfish" ;;
                net.bin)    engine="Akimbo (embedded)" ;;
                akimbo*)    engine="Akimbo (external)" ;;
                viridithas*)engine="Viridithas" ;;
                *.nnue*)    engine="NNUE" ;;
                *.bin)      engine="Binary" ;;
                *)          engine="Unknown" ;;
            esac
            printf "  %-40s %8s  [%s]\n" "$name" "$size" "$engine"
        done
    else
        echo "  (no networks directory)"
    fi

# Benchmark with a specific external network file
bench-net net_path depth="12":
    #!/usr/bin/env bash
    set -euo pipefail
    echo "🧪 Benchmarking with network: {{net_path}}"
    RUSTFLAGS="-C target-cpu=native" cargo run --release -- bench -d {{depth}} --net "{{net_path}}"

# ──────────────────────────────────────────────────────────────
# Install KishMat — bundles everything into a single package
# Book + NNUE network are compiled into the binary (fastest: direct memory access)
# - macOS:   .app bundle in ~/Applications with CLI engine + updater
# - Linux:   ~/.local/bin + .desktop entry
# - Windows: %LOCALAPPDATA%/KishMat with Start Menu shortcut
# ──────────────────────────────────────────────────────────────
install:
    #!/usr/bin/env bash
    set -euo pipefail

    echo "🔨 Building all KishMat components (optimized release)..."
    RUSTFLAGS="-C target-cpu=native" cargo build --release --workspace

    OS="$(uname -s)"
    case "$OS" in
        Darwin)
            echo "🍎 Installing for macOS..."

            # CLI binaries → ~/.local/bin
            mkdir -p "$HOME/.local/bin"
            cp target/release/kishmat     "$HOME/.local/bin/kishmat"
            cp target/release/kishmat-ui  "$HOME/.local/bin/kishmat-ui"
            if [ -f target/release/kishmat-updater ]; then
                cp target/release/kishmat-updater "$HOME/.local/bin/kishmat-updater"
            fi
            chmod +x "$HOME/.local/bin/kishmat" "$HOME/.local/bin/kishmat-ui"

            # PATH hint
            if ! echo "$PATH" | grep -q "$HOME/.local/bin"; then
                echo ""
                echo "⚠️  Add ~/.local/bin to your PATH:"
                echo '  echo '\''export PATH="$HOME/.local/bin:$PATH"'\'' >> ~/.zshrc'
            fi

            # ── macOS .app bundle ──
            APP_DIR="$HOME/Applications/KishMat.app"
            rm -rf "$APP_DIR"
            mkdir -p "$APP_DIR/Contents/MacOS"
            mkdir -p "$APP_DIR/Contents/Resources"

            # Bundle ALL binaries into the .app
            cp target/release/kishmat-ui "$APP_DIR/Contents/MacOS/KishMat"
            cp target/release/kishmat    "$APP_DIR/Contents/MacOS/kishmat-cli"
            if [ -f target/release/kishmat-updater ]; then
                cp target/release/kishmat-updater "$APP_DIR/Contents/MacOS/kishmat-updater"
            fi
            chmod +x "$APP_DIR/Contents/MacOS/"*

            # Icon — create .icns from logo PNG (macOS has sips + iconutil)
            cp logo-nobg.png "$APP_DIR/Contents/Resources/icon.png"
            ICNS_REF="KishMat"
            if command -v sips &>/dev/null && command -v iconutil &>/dev/null; then
                ICONSET="$APP_DIR/Contents/Resources/icon.iconset"
                mkdir -p "$ICONSET"
                for sz in 16 32 64 128 256 512; do
                    sips -z $sz $sz logo-nobg.png --out "$ICONSET/icon_${sz}x${sz}.png" &>/dev/null
                    double=$((sz * 2))
                    sips -z $double $double logo-nobg.png --out "$ICONSET/icon_${sz}x${sz}@2x.png" &>/dev/null
                done
                if iconutil -c icns "$ICONSET" -o "$APP_DIR/Contents/Resources/KishMat.icns" 2>/dev/null; then
                    ICNS_REF="KishMat"
                fi
                rm -rf "$ICONSET"
            fi

            # Info.plist — complete macOS app manifest
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
        <key>LSMinimumSystemVersion</key>
        <string>12.0</string>
        <key>NSSupportsAutomaticGraphicsSwitching</key>
        <true/>
    </dict>
    </plist>
    PLIST

            echo ""
            echo "✅ Installed:"
            echo "   CLI engine:  ~/.local/bin/kishmat"
            echo "   GUI app:     ~/Applications/KishMat.app"
            echo "   Updater:     ~/.local/bin/kishmat-updater"
            echo ""
            echo "   .app bundle contains: GUI + CLI engine + updater"
            ;;

        Linux)
            echo "🐧 Installing for Linux..."

            mkdir -p "$HOME/.local/bin"
            cp target/release/kishmat     "$HOME/.local/bin/kishmat"
            cp target/release/kishmat-ui  "$HOME/.local/bin/kishmat-ui"
            if [ -f target/release/kishmat-updater ]; then
                cp target/release/kishmat-updater "$HOME/.local/bin/kishmat-updater"
            fi
            chmod +x "$HOME/.local/bin/kishmat" "$HOME/.local/bin/kishmat-ui"

            if ! echo "$PATH" | grep -q "$HOME/.local/bin"; then
                echo ""
                echo "⚠️  Add ~/.local/bin to your PATH:"
                echo '  echo '\''export PATH="$HOME/.local/bin:$PATH"'\'' >> ~/.bashrc'
            fi

            # Icon (multiple sizes for HiDPI)
            for sz in 48 64 128 256; do
                ICON_DIR="$HOME/.local/share/icons/hicolor/${sz}x${sz}/apps"
                mkdir -p "$ICON_DIR"
                if command -v convert &>/dev/null; then
                    convert logo-nobg.png -resize ${sz}x${sz} "$ICON_DIR/kishmat.png" 2>/dev/null || \
                        cp logo-nobg.png "$ICON_DIR/kishmat.png"
                else
                    cp logo-nobg.png "$ICON_DIR/kishmat.png"
                fi
            done

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
    StartupWMClass=KishMat Chess
    DESKTOP

            # Refresh icon cache
            gtk-update-icon-cache "$HOME/.local/share/icons/hicolor" 2>/dev/null || true

            echo ""
            echo "✅ Installed:"
            echo "   CLI engine: ~/.local/bin/kishmat"
            echo "   GUI:        ~/.local/bin/kishmat-ui"
            echo "   Updater:    ~/.local/bin/kishmat-updater"
            echo "   Desktop:    ~/.local/share/applications/kishmat.desktop"
            ;;

        MINGW*|MSYS*|CYGWIN*)
            echo "🪟 Installing for Windows..."

            INSTALL_DIR="$LOCALAPPDATA/KishMat"
            mkdir -p "$INSTALL_DIR/bin"
            cp target/release/kishmat.exe     "$INSTALL_DIR/bin/kishmat.exe"
            cp target/release/kishmat-ui.exe  "$INSTALL_DIR/bin/kishmat-ui.exe"
            if [ -f target/release/kishmat-updater.exe ]; then
                cp target/release/kishmat-updater.exe "$INSTALL_DIR/bin/kishmat-updater.exe"
            fi
            cp logo-nobg.png "$INSTALL_DIR/icon.png"

            # Create Start Menu shortcut via PowerShell
            powershell.exe -NoProfile -Command "
                \$WshShell = New-Object -ComObject WScript.Shell;
                \$StartMenu = [System.IO.Path]::Combine(\$env:APPDATA, 'Microsoft\\Windows\\Start Menu\\Programs');
                \$Shortcut = \$WshShell.CreateShortcut(\"\$StartMenu\\KishMat Chess.lnk\");
                \$Shortcut.TargetPath = '$INSTALL_DIR\\bin\\kishmat-ui.exe';
                \$Shortcut.WorkingDirectory = '$INSTALL_DIR\\bin';
                \$Shortcut.Description = 'KishMat Chess - The First Arabian Chess Engine';
                \$Shortcut.Save()
            " 2>/dev/null && echo "   ✅ Start Menu shortcut created" || echo "   ⚠️  Start Menu shortcut skipped"

            echo ""
            echo "✅ Installed to: $INSTALL_DIR\\bin"
            echo "⚠️  Add to PATH: Settings → System → Environment Variables → Path → Add:"
            echo "   $INSTALL_DIR\\bin"
            ;;

        *)
            echo "❌ Unsupported OS: $OS"
            echo "   Manually copy target/release/kishmat and target/release/kishmat-ui to your PATH."
            exit 1
            ;;
    esac

    echo ""
    echo "🎉 KishMat installed successfully!"
    echo "   📦 Book & NNUE weights are compiled into the binary — no external files needed."

# Uninstall KishMat from all platforms
uninstall:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "Removing KishMat..."

    # macOS / Linux
    rm -f "$HOME/.local/bin/kishmat" "$HOME/.local/bin/kishmat-ui" "$HOME/.local/bin/kishmat-updater"
    rm -rf "$HOME/Applications/KishMat.app"
    rm -f "$HOME/.local/share/applications/kishmat.desktop"
    rm -f "$HOME/.local/share/icons/hicolor/"{48x48,64x64,128x128,256x256}"/apps/kishmat.png" 2>/dev/null || true
    gtk-update-icon-cache "$HOME/.local/share/icons/hicolor" 2>/dev/null || true

    # Windows
    if [ -n "${LOCALAPPDATA:-}" ]; then
        rm -rf "$LOCALAPPDATA/KishMat" 2>/dev/null || true
        powershell.exe -NoProfile -Command "
            \$StartMenu = [System.IO.Path]::Combine(\$env:APPDATA, 'Microsoft\\Windows\\Start Menu\\Programs');
            Remove-Item \"\$StartMenu\\KishMat Chess.lnk\" -ErrorAction SilentlyContinue
        " 2>/dev/null || true
    fi

    echo "✅ KishMat uninstalled."
