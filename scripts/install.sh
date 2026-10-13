#!/bin/sh
# Install Big Lips for this user and fetch the game, as OldUnreal's installers do:
#   - builds Big Lips (with SDL3 controllers when SDL3 is installed),
#   - installs it to ~/.local/share/biglips with a `biglips` command and a menu entry,
#   - downloads the full version of Exploding Lips from the Internet Archive
#     (https://archive.org/details/exploding-lips-full, public domain mark), checks it and
#     unpacks it where the game finds it.
# `scripts/install.sh --uninstall` removes Big Lips again (the game data and saves stay).
set -e
cd "$(dirname "$0")/.."
prefix="${XDG_DATA_HOME:-$HOME/.local/share}/biglips"
bin="$HOME/.local/bin"
apps="${XDG_DATA_HOME:-$HOME/.local/share}/applications"

if [ "$1" = "--uninstall" ]; then
    rm -rf "$prefix" "$bin/biglips" "$apps/biglips.desktop"
    echo "Big Lips removed. The game data and settings in ${XDG_CONFIG_HOME:-$HOME/.config}/exploding-lips stay."
    exit 0
fi

features=""
if pkg-config --exists sdl3 2>/dev/null; then features="--features lips-desktop/sdl"; fi
echo "Building Big Lips..."
cargo build --release -p lips-desktop -p packbuilder $features

mkdir -p "$prefix" "$bin" "$apps"
cp target/release/biglips target/release/packbuilder "$prefix/"
ln -sf "$prefix/biglips" "$bin/biglips"
cat > "$apps/biglips.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Big Lips
Comment=Exploding Lips, remastered
Exec=$prefix/biglips
Terminal=false
Categories=Game;ActionGame;
EOF

# The game: downloaded once into the config folder (skipped when already there).
"$prefix/biglips" --install

echo
echo "Installed. Start \"Big Lips\" from the menu, or run: biglips"
case ":$PATH:" in *":$bin:"*) ;; *) echo "(Add $bin to your PATH to use the biglips command.)" ;; esac
